import SwiftUI
#if os(iOS)
import UIKit
#else
import AppKit
#endif

func lsImpact() {
    #if os(iOS)
    UIImpactFeedbackGenerator(style: .light).impactOccurred()
    #endif
}

func lsHideKeyboard() {
    #if os(iOS)
    UIApplication.shared.sendAction(
        #selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
    #endif
}

func lsCopyToPasteboard(_ s: String) {
    #if os(iOS)
    UIPasteboard.general.string = s
    #else
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(s, forType: .string)
    #endif
}

// iOS/macOS modifier shims so the same views serve both apps (phase 6).
extension View {
    @ViewBuilder func lsInlineTitle() -> some View {
        #if os(iOS)
        navigationBarTitleDisplayMode(.inline)
        #else
        self
        #endif
    }

    @ViewBuilder func lsGroupedList() -> some View {
        #if os(iOS)
        listStyle(.insetGrouped)
        #else
        listStyle(.inset)
        #endif
    }

    @ViewBuilder func lsURLField() -> some View {
        #if os(iOS)
        keyboardType(.URL).textInputAutocapitalization(.never)
        #else
        self
        #endif
    }
}
