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

            Text(caption)
                .font(.pocket(.footnote))
                .foregroundStyle(PocketColor.textSecondary)

            Spacer()

            Button(action: onPauseResume) {
                Image(systemName: pauseResumeIcon)
            }
            .buttonStyle(.plain)
            Button(action: onCancel) {
                Image(systemName: "xmark")
            }
            .buttonStyle(.plain)
            .foregroundStyle(PocketColor.textTertiary)
        }
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
            let eta = bps > 0 ? (total - done) / max(bps, 1) : 0
            return "\(Self.gb(done)) of \(Self.gb(total)) · \(Self.mbps(bps)) · \(Self.duration(eta)) left"
        case .paused(let done, let total):
            return "Paused · \(Self.gb(done)) of \(Self.gb(total))"
        case .verifying:
            return "Checking file…"
        }
    }

    private static func gb(_ bytes: Int64) -> String {
        String(format: "%.1f GB", Double(bytes) / 1_000_000_000)
    }

    private static func mbps(_ bytesPerSecond: Int64) -> String {
        String(format: "%.0f MB/s", Double(bytesPerSecond) / 1_000_000)
    }

    private static func duration(_ seconds: Int64) -> String {
        let minutes = max(seconds / 60, 1)
        return minutes == 1 ? "1 min" : "\(minutes) min"
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
