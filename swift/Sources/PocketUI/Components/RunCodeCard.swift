import SwiftUI

/// AGT-13: the "Run this?" card shown before the model's code runs. The code
/// sits in a monospaced block the user can edit in place; nothing runs until
/// Run is tapped. Strings stay English until the catalog pass (like the
/// AGT-12 chip); every control carries an accessibility label.
public struct RunCodeCard: View {
    private let language: String
    private let place: String
    private let code: String
    private let offersAlwaysAllow: Bool
    private let onMac: Bool
    private let onRun: (_ code: String, _ alwaysAllow: Bool) -> Void
    private let onDeny: () -> Void

    @State private var draft: String
    @State private var editing = false
    @State private var alwaysAllow = false

    /// - Parameters:
    ///   - language: "JavaScript" or "Python".
    ///   - place: where it runs, already localised, e.g. "this iPhone".
    ///   - offersAlwaysAllow: the "always allow in this chat" switch. JavaScript
    ///     only: Python asks every time.
    ///   - onMac: the code goes to the paired Mac ("Run this on your Mac?").
    public init(language: String, place: String, code: String, offersAlwaysAllow: Bool = true,
                onMac: Bool = false,
                onRun: @escaping (_ code: String, _ alwaysAllow: Bool) -> Void,
                onDeny: @escaping () -> Void) {
        self.onMac = onMac
        self.language = language
        self.place = place
        self.code = code
        self.offersAlwaysAllow = offersAlwaysAllow
        self.onRun = onRun
        self.onDeny = onDeny
        _draft = State(initialValue: code)
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: PocketSpacing.s12) {
            VStack(alignment: .leading, spacing: PocketSpacing.s2) {
                Text(onMac ? LocalizedStringKey("Run this on your Mac?") : "Run this?")
                    .font(.pocket(.headline))
                    .foregroundStyle(PocketColor.textPrimary)
                    .accessibilityAddTraits(.isHeader)
                Text(onMac
                     ? String(localized: "\(language) on \(place), in a sandbox with no network and no access to your files.", bundle: .module)
                     : String(localized: "\(language) on \(place). No network, no files.", bundle: .module))
                    .font(.pocket(.footnote))
                    .foregroundStyle(PocketColor.textSecondary)
            }

            if editing {
                TextEditor(text: $draft)
                    .font(.pocket(.mono))
                    .scrollContentBackground(.hidden)
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    #endif
                    .padding(PocketSpacing.s8)
                    .frame(minHeight: 140, maxHeight: 280)
                    .background(PocketColor.bgSunken, in: RoundedRectangle(cornerRadius: PocketRadius.chip, style: .continuous))
                    .accessibilityLabel("Code to run")
            } else {
                ScrollView(.vertical) {
                    CodeBlock(code: draft, language: language.lowercased())
                }
                .frame(maxHeight: 280)
                .fixedSize(horizontal: false, vertical: true)
            }

            if offersAlwaysAllow {
                Toggle(isOn: $alwaysAllow) {
                    Text("Always allow \(language) in this chat")
                        .font(.pocket(.footnote))
                        .foregroundStyle(PocketColor.textPrimary)
                }
                .tint(PocketColor.accent)
                .accessibilityLabel("Always allow \(language) in this chat")
                .accessibilityHint("Skips this card for the rest of this chat until you quit the app")
            }

            HStack(spacing: PocketSpacing.s8) {
                Button(action: onDeny) {
                    Text("Don't run").frame(maxWidth: .infinity)
                }
                .buttonStyle(PocketSecondaryButtonStyle())
                .accessibilityLabel("Don't run this code")

                Button { editing.toggle() } label: {
                    Text(editing ? LocalizedStringKey("Done") : "Edit").frame(maxWidth: .infinity)
                }
                .buttonStyle(PocketSecondaryButtonStyle())
                .accessibilityLabel(editing ? LocalizedStringKey("Finish editing the code") : "Edit the code")

                Button { onRun(draft, offersAlwaysAllow && alwaysAllow) } label: {
                    Text("Run").frame(maxWidth: .infinity)
                }
                .buttonStyle(PocketPrimaryButtonStyle())
                .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .accessibilityLabel("Run this code")
            }
            #if os(iOS)
            .frame(minHeight: 44)
            #endif
        }
        .padding(PocketSpacing.s16)
        .background(PocketColor.bgRaised, in: RoundedRectangle(cornerRadius: PocketRadius.card, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: PocketRadius.card, style: .continuous).strokeBorder(PocketColor.accent))
        .accessibilityElement(children: .contain)
    }
}

/// AGT-13: "Running code…" with the Stop control, while a script runs.
public struct RunningCodeRow: View {
    private let onStop: () -> Void

    public init(onStop: @escaping () -> Void) {
        self.onStop = onStop
    }

    public var body: some View {
        HStack(spacing: PocketSpacing.s8) {
            ProgressView().controlSize(.small)
            Text("Running code…")
                .font(.pocket(.footnote))
                .foregroundStyle(PocketColor.textSecondary)
            Spacer()
            Button(action: onStop) {
                Text("Stop")
                    .font(.pocket(.footnote))
                    .fontWeight(.semibold)
                    .foregroundStyle(PocketColor.statusBad)
                    #if os(iOS)
                    .frame(minWidth: 44, minHeight: 44)
                    #endif
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Stop running code")
        }
        .accessibilityElement(children: .contain)
    }
}

/// AGT-13: a finished (or declined) code run as an expandable row: the code,
/// its output, any error and how long it took.
public struct RanCodeRow: View {
    private let language: String
    private let code: String
    private let output: String
    private let error: String
    private let durationMs: UInt64
    private let truncated: Bool
    private let denied: Bool
    private let onMac: Bool
    @State private var expanded = false

    public init(language: String, code: String, output: String, error: String,
                durationMs: UInt64, truncated: Bool, denied: Bool, onMac: Bool = false) {
        self.onMac = onMac
        self.language = language
        self.code = code
        self.output = output
        self.error = error
        self.durationMs = durationMs
        self.truncated = truncated
        self.denied = denied
    }

    private var title: String {
        if denied {
            return onMac ? String(localized: "Didn't run \(language) on your Mac", bundle: .module)
                         : String(localized: "Didn't run \(language)", bundle: .module)
        }
        if error.isEmpty {
            return onMac ? String(localized: "Ran \(language) on your Mac", bundle: .module)
                         : String(localized: "Ran \(language)", bundle: .module)
        }
        return onMac ? String(localized: "\(language) failed on your Mac", bundle: .module)
                     : String(localized: "\(language) failed", bundle: .module)
    }

    private var symbol: String {
        denied ? "hand.raised" : (error.isEmpty ? "terminal" : "exclamationmark.triangle")
    }

    public var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            VStack(alignment: .leading, spacing: PocketSpacing.s8) {
                CodeBlock(code: code, language: language.lowercased())
                if !output.isEmpty {
                    block("Output", output, PocketColor.textPrimary)
                    if truncated {
                        Text("Output was cut off at 20 KB.")
                            .font(.pocket(.caption))
                            .foregroundStyle(PocketColor.textTertiary)
                    }
                }
                if !error.isEmpty {
                    block("Error", error, PocketColor.statusBad)
                }
                if !denied {
                    Text("Took \(durationMs) ms")
                        .font(.pocket(.caption))
                        .foregroundStyle(PocketColor.textTertiary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.bottom, PocketSpacing.s8)
        } label: {
            Label(title, systemImage: symbol)
                #if os(iOS)
                .font(.pocket(.footnote))
                .frame(minHeight: 44)
                #else
                .font(.pocket(.caption))
                #endif
                .foregroundStyle(PocketColor.textSecondary)
                .lineLimit(1)
        }
        .tint(PocketColor.textTertiary)
        .accessibilityLabel(title)
        .accessibilityHint(expanded ? LocalizedStringKey("Collapse the code and its output") : "Show the code and its output")
    }

    private func block(_ heading: String, _ text: String, _ color: Color) -> some View {
        VStack(alignment: .leading, spacing: PocketSpacing.s2) {
            Text(heading)
                .font(.pocket(.caption))
                .foregroundStyle(PocketColor.textTertiary)
            Text(text)
                .font(.pocket(.mono))
                .foregroundStyle(color)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(PocketSpacing.s8)
                .background(PocketColor.bgSunken, in: RoundedRectangle(cornerRadius: PocketRadius.chip, style: .continuous))
        }
    }
}

#Preview("Run this?") {
    VStack(spacing: PocketSpacing.s16) {
        RunCodeCard(language: "JavaScript", place: "this iPhone",
                    code: "let f = 1;\nfor (let i = 2; i <= 17; i++) f *= i;\nf / 3 ** 9",
                    onRun: { _, _ in }, onDeny: {})
        RunningCodeRow(onStop: {})
        RanCodeRow(language: "JavaScript", code: "1 + 1", output: "=> 2", error: "",
                   durationMs: 3, truncated: false, denied: false)
    }
    .padding()
    .background(PocketColor.bgCanvas)
}
