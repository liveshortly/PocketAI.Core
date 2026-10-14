import SwiftUI

/// Typography tokens. System fonts only (no bundled fonts) so Dynamic Type
/// scales every style up to AX5.
public enum PocketTextStyle {
    case display
    case title
    case headline
    case body
    case callout
    case footnote
    case caption
    case mono
    case metric
    case reading
}

public extension Font {
    static func pocket(_ style: PocketTextStyle) -> Font {
        switch style {
        case .display:
            return .system(.largeTitle, design: .rounded, weight: .bold)
        case .title:
            return .system(.title2, design: .default, weight: .semibold)
        case .headline:
            return .system(.headline, design: .default, weight: .semibold)
        case .body:
            return .system(.body, design: .default, weight: .regular)
        case .callout:
            return .system(.callout, design: .default, weight: .regular)
        case .footnote:
            return .system(.footnote, design: .default, weight: .regular)
        case .caption:
            return .system(.caption, design: .default, weight: .medium)
        case .mono:
            return .system(.body, design: .monospaced, weight: .regular)
        case .metric:
            return .system(.footnote, design: .monospaced, weight: .medium)
        case .reading:
            return .system(.body, design: .serif, weight: .regular)
        }
    }
}
