import AVFoundation
import Foundation
#if canImport(FoundationModels)
import Speech
#endif

/// APL-12: on-device transcription via iOS/macOS 26's `SpeechAnalyzer` +
/// `SpeechTranscriber`, downloading the locale asset through
/// `AssetInventory` when needed (an Apple system download, not app
/// content - still worth an egress record, left to the caller since
/// `SpeechToTextEngine` has no egress hook of its own). Gated behind
/// `#if canImport(FoundationModels)`: that framework ships in the exact
/// same SDK generation as these Speech APIs, but unlike `Speech` itself
/// (present in every SDK), it reliably signals "building against an
/// iOS/macOS 26+ SDK" - `SpeechAnalyzer`/`SpeechTranscriber`/`AssetInventory`
/// are new API *within* the always-importable `Speech` module, so
/// `canImport(Speech)` can't be used to gate them (D63). Older SDKs (and
/// pre-26 OS at runtime) fall back to an explicit "not implemented" error
/// rather than silently returning nothing - the WhisperKit fallback path
/// (a third-party Core ML model) is a separate, larger follow-on.
public final class AppleSpeechToTextEngine: SpeechToTextEngine {
    public init() {}

    public func transcribe(audioPath: String, lang: String?, sink: TokenSink) throws {
        #if canImport(FoundationModels)
        guard #available(iOS 26.0, macOS 26.0, *) else {
            sink.onError(err: .Internal(message: "on-device speech-to-text needs iOS/macOS 26+ (WhisperKit fallback not implemented yet)"))
            return
        }
        let semaphore = DispatchSemaphore(value: 0)
        let box = ErrorBox()
        Task {
            do {
                try await Self.run(audioPath: audioPath, lang: lang, sink: sink)
            } catch {
                box.error = error
            }
            semaphore.signal()
        }
        semaphore.wait()
        if let error = box.error {
            throw PocketError.Internal(message: "\(error)")
        }
        #else
        sink.onError(err: .Internal(message: "on-device speech-to-text needs an iOS/macOS 26+ SDK (WhisperKit fallback not implemented yet)"))
        #endif
    }

    #if canImport(FoundationModels)
    private final class ErrorBox: @unchecked Sendable {
        var error: Error?
    }

    @available(iOS 26.0, macOS 26.0, *)
    private static func run(audioPath: String, lang: String?, sink: TokenSink) async throws {
        let requested = lang.flatMap { Locale(identifier: $0) } ?? Locale.current
        let locale = await SpeechTranscriber.supportedLocale(equivalentTo: requested) ?? Locale(identifier: "en-US")
        let transcriber = SpeechTranscriber(locale: locale, preset: .transcription)

        if await AssetInventory.status(forModules: [transcriber]) != .installed {
            if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
                try await request.downloadAndInstall()
            }
        }

        let audioFile = try AVAudioFile(forReading: URL(fileURLWithPath: audioPath))
        let analyzer = SpeechAnalyzer(modules: [transcriber])

        async let resultsTask: Void = {
            for try await result in transcriber.results where result.isFinal {
                sink.onDelta(delta: GenerateDelta(text: String(result.text.characters), reasoning: "", toolCalls: []))
            }
        }()

        try await analyzer.start(inputAudioFile: audioFile, finishAfterFile: true)
        try await resultsTask
        sink.onDone(stats: GenerateStats(promptTokens: 0, completionTokens: 0, ttftMs: 0, decodeTps: 0, finish: .stop))
    }
    #endif
}
