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

/// A LiveShortly session's icon: the LS mark, green while live (with a subtle
/// pulse, none under Reduce Motion), gray once ended. Status is never colour
/// alone — rows keep their "LIVE" badge / "ended …" text.
public struct LSSessionMark: View {
    let isLive: Bool
    var size: CGFloat = 34
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var dim = false

    public init(isLive: Bool, size: CGFloat = 34) {
        self.isLive = isLive
        self.size = size
    }

    public var body: some View {
        LSLogoShape()
            .fill(isLive ? PocketColor.statusOk : PocketColor.textTertiary, style: FillStyle(eoFill: true))
            .frame(width: size * 0.5, height: size * 0.5)
            .opacity(isLive && dim && !reduceMotion ? 0.55 : 1)
            .animation(
                isLive && !reduceMotion ? .easeInOut(duration: 1.2).repeatForever(autoreverses: true) : nil,
                value: dim)
            .frame(width: size, height: size)
            .background(
                RoundedRectangle(cornerRadius: size * 0.3)
                    .fill(isLive ? PocketColor.statusOk.opacity(0.12) : PocketColor.bgSunken)
            )
            .onAppear { dim = true }
            .accessibilityElement()
            .accessibilityLabel(isLive ? "LiveShortly session, live" : "LiveShortly session, ended")
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
