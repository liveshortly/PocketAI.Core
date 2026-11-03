import SwiftUI

public enum DownloadProgressState: Sendable {
    case downloading(bytesDone: Int64, bytesTotal: Int64, bytesPerSecond: Int64)
    case verifying
    case paused(bytesDone: Int64, bytesTotal: Int64)
}

/// C10: ring + "1.2 of 2.3 GB · 18 MB/s · 1 min left"; pause/resume/cancel.
public struct DownloadProgress: View {
    private let state: DownloadProgressState
    private let onPauseResume: () -> Void
    private let onCancel: () -> Void

    public init(state: DownloadProgressState, onPauseResume: @escaping () -> Void, onCancel: @escaping () -> Void) {
        self.state = state
        self.onPauseResume = onPauseResume
        self.onCancel = onCancel
    }

    public var body: some View {
        HStack(spacing: PocketSpacing.s12) {
            ZStack {
                Circle()
                    .stroke(PocketColor.lineHairline, lineWidth: 3)
                Circle()
                    .trim(from: 0, to: fraction)
                    .stroke(PocketColor.accent, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                    .rotationEffect(.degrees(-90))
            }
            .frame(width: 28, height: 28)
            .accessibilityHidden(true)

            Text(caption)
                .font(.pocket(.footnote))
                .foregroundStyle(PocketColor.textSecondary)
                .accessibilityLabel(Text("Download", bundle: .module))
                .accessibilityValue(caption)

            Spacer()

            // UI-132: icon-only controls had no names ("pause.fill") and
            // ~20pt targets.
            Button(action: onPauseResume) {
                Image(systemName: pauseResumeIcon)
                    .frame(minWidth: 44, minHeight: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(pauseResumeLabel)
            Button(action: onCancel) {
                Image(systemName: "xmark")
                    .frame(minWidth: 44, minHeight: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(PocketColor.textSecondary)
            .accessibilityLabel(Text("Cancel download", bundle: .module))
        }
    }

    private var pauseResumeLabel: Text {
        if case .paused = state { return Text("Resume download", bundle: .module) }
        return Text("Pause download", bundle: .module)
    }

    private var fraction: Double {
        switch state {
        case .downloading(let done, let total, _), .paused(let done, let total):
            return total > 0 ? Double(done) / Double(total) : 0
        case .verifying:
            return 1
        }
    }

    private var pauseResumeIcon: String {
        switch state {
        case .downloading: "pause.fill"
        case .paused: "play.fill"
        case .verifying: "pause.fill"
        }
    }

    private var caption: String {
        switch state {
        case .downloading(let done, let total, let bps):
            // No speed yet: "0 MB/s · 1 min left" was a made-up number.
            guard bps > 0 else {
                return String(localized: "\(Self.gb(done)) of \(Self.gb(total))", bundle: .module)
            }
            let eta = (total - done) / bps
            return String(localized: "\(Self.gb(done)) of \(Self.gb(total)) · \(Self.mbps(bps)) · \(Self.duration(eta)) left", bundle: .module)
        case .paused(let done, let total):
            return String(localized: "Paused · \(Self.gb(done)) of \(Self.gb(total))", bundle: .module)
        case .verifying:
            return String(localized: "Checking file…", bundle: .module)
        }
    }

    private static func gb(_ bytes: Int64) -> String {
        String(localized: "\(String(format: "%.1f", Double(bytes) / 1_000_000_000)) GB", bundle: .module)
    }

    private static func mbps(_ bytesPerSecond: Int64) -> String {
        String(localized: "\(String(format: "%.0f", Double(bytesPerSecond) / 1_000_000)) MB/s", bundle: .module)
    }

    private static func duration(_ seconds: Int64) -> String {
        let minutes = max(seconds / 60, 1)
        return String(localized: "\(minutes) min", bundle: .module)
    }
}

#Preview("Light") {
    VStack(spacing: PocketSpacing.s16) {
        DownloadProgress(state: .downloading(bytesDone: 1_200_000_000, bytesTotal: 2_300_000_000, bytesPerSecond: 18_000_000), onPauseResume: {}, onCancel: {})
        DownloadProgress(state: .paused(bytesDone: 1_200_000_000, bytesTotal: 2_300_000_000), onPauseResume: {}, onCancel: {})
        DownloadProgress(state: .verifying, onPauseResume: {}, onCancel: {})
    }
    .padding()
    .background(PocketColor.bgCanvas)
}

#Preview("Dark") {
    DownloadProgress(state: .downloading(bytesDone: 1_200_000_000, bytesTotal: 2_300_000_000, bytesPerSecond: 18_000_000), onPauseResume: {}, onCancel: {})
        .padding()
        .background(PocketColor.bgCanvas)
        .preferredColorScheme(.dark)
}
