import AVFoundation
import Foundation

/// APL-13 fallback path: `AVSpeechSynthesizer` writes directly to a wav
/// file via `write(_:toBufferCallback:)` - no download, works on every
/// supported OS version. The primary path (FluidAudio Kokoro, a Core ML
/// voice model) isn't implemented here - that's a separate, larger
/// third-party-dependency follow-on; this fallback is itself a complete,
/// real implementation, not a stub standing in for it.
public final class AppleTextToSpeechEngine: NSObject, TextToSpeechEngine, @unchecked Sendable {
    private let synthesizer = AVSpeechSynthesizer()

    public override init() {
        super.init()
    }

    public func synthesize(text: String, voice: String, outPath: String) throws {
        let utterance = AVSpeechUtterance(string: text)
        if !voice.isEmpty {
            utterance.voice = AVSpeechSynthesisVoice(identifier: voice) ?? AVSpeechSynthesisVoice(language: voice)
        }

        let semaphore = DispatchSemaphore(value: 0)
        let state = WriteState()

        synthesizer.write(utterance) { buffer in
            guard let pcmBuffer = buffer as? AVAudioPCMBuffer else { return }
            if pcmBuffer.frameLength == 0 {
                semaphore.signal()
                return
            }
            do {
                let file = try state.file(for: pcmBuffer.format, outPath: outPath)
                try file.write(from: pcmBuffer)
            } catch {
                state.error = error
                semaphore.signal()
            }
        }

        semaphore.wait()
        if let error = state.error {
            throw PocketError.Internal(message: "\(error)")
        }
    }

    /// Holds the lazily-created output file (format isn't known until the
    /// first buffer arrives) across the synthesis callback.
    private final class WriteState: @unchecked Sendable {
        var error: Error?
        private var audioFile: AVAudioFile?

        func file(for format: AVAudioFormat, outPath: String) throws -> AVAudioFile {
            if let audioFile { return audioFile }
            let settings: [String: Any] = [
                AVFormatIDKey: kAudioFormatLinearPCM,
                AVSampleRateKey: format.sampleRate,
                AVNumberOfChannelsKey: format.channelCount,
                AVLinearPCMBitDepthKey: 16,
                AVLinearPCMIsFloatKey: false,
            ]
            let file = try AVAudioFile(forWriting: URL(fileURLWithPath: outPath), settings: settings)
            audioFile = file
            return file
        }
    }
}
