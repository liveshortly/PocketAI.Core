import SwiftUI
import PocketUI

/// The LiveShortly mark — a solid panel beside an outlined one — traced from
/// the brand source `liveshortly-server/web/components/BrandMark.tsx`
/// (32-unit grid). Drawn as a Shape so it is a vector template: tint it with
/// `.foregroundStyle`.
public struct LSLogoShape: Shape {
    public init() {}

    public func path(in rect: CGRect) -> Path {
        // Mark bounding box on the 32 grid is 19 × 17 (stroke included).
        let s = min(rect.width / 19, rect.height / 17)
        let ox = rect.midX - 19 * s / 2, oy = rect.midY - 17 * s / 2
        func r(_ x: CGFloat, _ y: CGFloat, _ w: CGFloat, _ h: CGFloat, _ c: CGFloat) -> Path {
            Path(roundedRect: CGRect(x: ox + x * s, y: oy + y * s, width: w * s, height: h * s),
                 cornerRadius: c * s)
        }
        var p = r(0, 1, 6.5, 15, 1.5)               // solid panel
        p.addPath(r(8.5, 0, 10.5, 17, 2.5))         // outlined panel, outer edge
        p.addPath(r(10.5, 2, 6.5, 13, 0.5))         // … inner edge (even-odd hole)
        return p
    }
}

public extension Image {
    /// The LS mark as a template vector image (`ls-logo` in LSLogo.xcassets,
    /// same geometry as `LSLogoShape`) — for menus/labels, which can't host a Shape.
    static let lsLogo = Image("ls-logo", bundle: .module)
}

/// A LiveShortly session's icon (UI-34): the PocketAI chip mark, Live motion
/// while running, grey once ended. Status is never colour alone — rows keep
/// their "LIVE" badge / "ended …" text.
public struct LSSessionMark: View {
    let isLive: Bool
    var isStarting = false
    var size: CGFloat = 34

    public init(isLive: Bool, size: CGFloat = 34) {
        self.isLive = isLive
        self.size = size
    }

    /// Starting… (loading pulse) / Live / ended, from the session's status.
    public init(session: LSSession, size: CGFloat = 34) {
        self.isLive = session.isLive
        self.isStarting = session.isStarting
        self.size = size
    }

    public var body: some View {
        PocketMarkView(state: isStarting ? .loading : isLive ? .live : .offline, size: size)
            .accessibilityElement()
            .accessibilityLabel(isStarting ? "LiveShortly session, starting"
                : isLive ? "LiveShortly session, live" : "LiveShortly session, ended")
    }
}

#Preview("LS session mark") {
    HStack(spacing: 16) {
        LSSessionMark(isLive: true)
        LSSessionMark(isLive: false)
        LSSessionMark(isLive: true, size: 22)
        LSSessionMark(isLive: false, size: 22)
    }
    .padding()
}
