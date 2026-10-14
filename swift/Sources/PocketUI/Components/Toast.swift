import SwiftUI

/// C18: a glass capsule toast, top-center, auto-dismissing after 3s with
/// an optional Undo action.
public struct ToastData: Identifiable, Equatable {
    public let id = UUID()
    public let message: String
    public let undo: (() -> Void)?

    public init(message: String, undo: (() -> Void)? = nil) {
        self.message = message
        self.undo = undo
    }

    public static func == (lhs: ToastData, rhs: ToastData) -> Bool {
        lhs.id == rhs.id
    }
}

public struct ToastView: View {
    private let toast: ToastData

    public init(_ toast: ToastData) {
        self.toast = toast
    }

    public var body: some View {
        HStack(spacing: PocketSpacing.s12) {
            Text(toast.message)
                .font(.pocket(.callout))
                .foregroundStyle(PocketColor.textPrimary)
            if let undo = toast.undo {
                Button("Undo", action: undo)
                    .font(.pocket(.callout))
                    .foregroundStyle(PocketColor.accent)
            }
        }
        .padding(.horizontal, PocketSpacing.s16)
        .padding(.vertical, PocketSpacing.s12)
        .background(.regularMaterial, in: Capsule())
        .shadow(radius: 8, y: 2)
    }
}

/// Presents at most one toast at a time, auto-dismissing after `duration`.
public struct ToastOverlay: ViewModifier {
    @Binding var toast: ToastData?
    var duration: Duration = .seconds(3)

    public func body(content: Content) -> some View {
        content.overlay(alignment: .top) {
            if let toast {
                ToastView(toast)
                    .padding(.top, PocketSpacing.s8)
                    .transition(.move(edge: .top).combined(with: .opacity))
                    .task(id: toast.id) {
                        try? await Task.sleep(for: duration)
                        if self.toast?.id == toast.id {
                            self.toast = nil
                        }
                    }
            }
        }
        .animation(.spring(duration: 0.3), value: toast)
    }
}

public extension View {
    func pocketToast(_ toast: Binding<ToastData?>) -> some View {
        modifier(ToastOverlay(toast: toast))
    }
}

#Preview("Light") {
    VStack {
        ToastView(ToastData(message: "Model deleted", undo: {}))
        ToastView(ToastData(message: "Copied to clipboard"))
    }
    .padding()
    .background(PocketColor.bgCanvas)
}

#Preview("Dark") {
    ToastView(ToastData(message: "Model deleted", undo: {}))
        .padding()
        .background(PocketColor.bgCanvas)
        .preferredColorScheme(.dark)
}
