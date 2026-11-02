# i18n inventory

What is localized today, and exactly what is left in the two apps.

Generated with Apple's own extractor, so the line numbers are the compiler's,
not a grep's:

```sh
xcrun xcstringstool extract --SwiftUI --modern-localizable-strings \
  --all-potential-swift-keys $(find Sources -name '*.swift') -o /tmp/sd
```

## Status

| Target | Catalog | Locales | Source converted |
|---|---|---|---|
| `PocketAI.Core` → `PocketUI` | `swift/Sources/PocketUI/Resources/Localizable.xcstrings` | en + hi, es, fr, de, ja, zh-Hans, pt-BR | **yes — all 54 keys, `swift build` + `swift test` green** |
| `PocketAI.Iphone` | `Resources/Localizable.xcstrings` | en only | no — 264 sites pending |
| `PocketAI.Mac` | `Resources/Localizable.xcstrings` | en only | no — 236 sites pending |

## Already done, shared by both apps

Everything in `PocketUI` is localized and needs no work in the apps. The app
targets get these for free, in all seven locales, through the component API:

`PocketMarkView` (4 accessibility states) · `ThreadBeamView` · `Composer`
(attach menu, placeholder, send/stop/mic labels and hints) · `MessageView`
("Thinking…", "Thought for a moment") · `CodeBlock` ("CODE", "Copy",
"Copied") · `FitBadge` (4 verdicts) · `DownloadProgress` (progress caption,
"Checking file…", GB/MB-s/min units) · `PulseStrip` ("Ready", "Offline",
"On %@", tok/s, a11y label and hint) · `PulseDetailSheet` ("Memory",
"Thermal state", "Last reply", "Loaded models", "Eject", "Network Proof",
RAM caption) · `DeviceRow` ("Connected", "Revoke") · `Toast` ("Undo") ·
`UserBubble` ("Attached image") · `QRScannerView` (simulator notice).

**Not** localized in `PocketUI`, deliberately:

- `Sources/PocketImageGenVendor/` — vendored third-party code, left alone.
- `Sources/PocketUI/Previews/` (`ComponentGallery`, `TokenGallery`) — a
  developer surface behind `-PocketComponentGallery`, with fixture captions
  like `PocketMark (C01)`. Not user-facing.
- `Sources/PocketLiveShortly/` — out of the brief's scope, and currently
  uncommitted work in progress. It has user-facing strings and no resource
  bundle, so it will need `defaultLocalization` plumbing of its own.
- The product name `PocketAI` itself, and strings the caller supplies
  (`SettingsRow(title:)`, `StatTile(label:)`, `EmptyState(title:)`,
  `ModelRow(primaryActionLabel:)`, `ToastData(message:)`,
  `FitBadge(reason:)`, `DeviceRow(lastSeenText:)`). Those literals live in the
  apps and are counted in the per-app tables below.

## How the two columns differ

The apps need less work than the raw literal count suggests, because an app
target's `Text("Delete chat")` is already a `LocalizedStringKey` lookup against
`Bundle.main`. Those sites need **no code change at all** — only a key in the
catalog, which is why they are seeded in English already.

What does need editing is the second column: literals sitting in a plain
`String` position — a computed `var label: String`, an error message assigned
to `@State`, an `UIAccessibility` announcement, or a `String` argument to a
PocketUI component. Those stay English forever unless they become
`String(localized:)`.

Note the asymmetry with the library: `PocketUI` must pass `bundle: .module` on
every lookup, because a package target's default bundle is the *app* bundle and
the lookup fails silently. App targets must **not** pass a bundle.

### Filling in the interpolated keys

The seeded catalogs hold only the keys the lightweight extractor could resolve
exactly. It cannot type-check interpolations, so `"Download failed (\(code))"`
comes out as `Download failed (%arg)` — the real key is `%lld` or `%@`
depending on the type. 50 iOS and 19 Mac keys are in that state and are
deliberately left out rather than written in wrong.

They land correctly on the first successful build, because the build emits
typed `.stringsdata`:

```sh
xcrun xcstringstool sync Resources/Localizable.xcstrings \
  --stringsdata $(find "$DERIVED_DATA" -name '*.stringsdata' -path '*PocketAI.build*')
```

Xcode does this automatically when the catalog is a member of the target, which
it now is.

## Blocker: the iPhone app does not currently build

Independent of localization. `AppModel.swift`, `SettingsView.swift`,
`HomeView.swift`, `ProjectDetailView.swift` and `ChatsListView.swift` all
`import PocketLiveShortly`, but `PocketAI.Iphone/project.yml` declares only
`PocketKit`, `PocketUI`, `PocketImageGen` and `PocketMLX`, so the link fails:

```
Undefined symbols for architecture arm64:
  "type metadata accessor for PocketLiveShortly.LSLastEvent", referenced from:
      outlined destroy of PocketLiveShortly.LSSessionsSection in ChatsListView.o
```

The product dependency has to be added to `project.yml` before any app-level
localization can be compiled or seen at runtime.

## Pending sites, per file

The second column is `--all-potential-swift-keys` output, which is deliberately
over-inclusive: it cannot type-check, so a handful of rows are not user-facing
at all (regexes in `DiagnosticsReport`, MIME types, `pocket.conversationsChanged`,
`en-US`, filename templates like `dictation_%arg.m4a`, `#Preview` prompts in
`ImageGenerationSheet`). Treat the list as the set to *review*, not a set to
convert blindly. Anything genuinely not user-facing should be left alone.

### PocketAI.Iphone

264 string sites still need a code change; 433 literals already sit in a `LocalizedStringKey` / `LocalizedStringResource` position and resolve from `Bundle.main` as soon as the key exists in the catalog (these are already seeded in English).

| File | Already localizable | Sites to convert (line numbers) |
|---|---:|---|
| `Sources/AppModel.swift` | 0 | 101 |
| `Sources/AskPocketAIIntent.swift` | 0 | 12, 13, 27, 44 |
| `Sources/CameraCaptureView.swift` | 0 | — |
| `Sources/ChatView.swift` | 78 | 174, 176, 357, 376, 419, 419, 455, 525, 526, 527, 818, 832, 923, 932, 1115, 1118, 1120, 1232, 1232, 1248, 1248, 1249, 1249, 1344, 1347, 1355, 1379, 1379, 1384 |
| `Sources/ChatsListView.swift` | 46 | 84, 85, 373, 438, 439, 440, 582, 582 |
| `Sources/DevicesView.swift` | 35 | 19, 40, 41, 97, 98, 102, 103, 319, 363, 436, 522, 535, 541, 560, 563, 571 |
| `Sources/DiagnosticsPreviewView.swift` | 14 | 204, 205 |
| `Sources/DiagnosticsReport.swift` | 0 | 21, 153, 160, 167, 174 |
| `Sources/HomeView.swift` | 8 | 172, 174, 175, 185, 210, 210, 223, 252, 252, 265, 425, 426, 464, 464, 493, 494, 505, 506, 534, 642, 646, 666, 670, 670, 671, 678, 682, 684, 690, 690, 701 |
| `Sources/ImageGenerationSheet.swift` | 39 | 70, 81, 123, 159, 191, 469, 479, 479, 547, 548, 551, 552, 557, 563 |
| `Sources/ModelPickerView.swift` | 5 | 17, 18, 24, 25, 63, 64 |
| `Sources/ModelsView.swift` | 48 | 19, 21, 29, 58, 226, 399, 411, 505, 506, 507, 508, 519, 519, 524, 525, 564, 702, 703, 704, 705, 730, 760, 881, 898, 902, 911, 926, 927, 929, 931, 978, 1000, 1003, 1006, 1048, 1051, 1052, 1181, 1181 |
| `Sources/NetworkMonitor.swift` | 0 | — |
| `Sources/NewProjectSheet.swift` | 11 | — |
| `Sources/OnboardingFlow.swift` | 22 | 145, 146, 150, 151, 155, 156, 224, 355, 487, 488, 489, 535, 536, 609, 610, 611 |
| `Sources/PerfDiagnostics.swift` | 0 | 141 |
| `Sources/PermissionPrimer.swift` | 1 | 130, 131, 142, 143, 144, 155, 156, 157 |
| `Sources/PocketAIApp.swift` | 0 | 30, 49, 55, 110, 118, 119, 134, 151 |
| `Sources/PocketAIShortcuts.swift` | 0 | 8, 9, 11, 17, 18, 20, 26, 27, 29 |
| `Sources/ProjectDetailView.swift` | 46 | 311, 526, 563 |
| `Sources/RootTabView.swift` | 6 | — |
| `Sources/SettingsDestinations.swift` | 29 | 70, 116, 123, 129, 130, 189, 220, 291, 309, 326, 326, 327, 327 |
| `Sources/SettingsView.swift` | 21 | 37, 44, 51, 51, 51, 61, 67, 73, 83, 91, 98, 110, 136, 171, 190, 194 |
| `Sources/SharedInbox.swift` | 0 | 164, 166 |
| `Sources/SharedInboxIngestSheet.swift` | 14 | 49, 53, 68 |
| `Sources/SourceSheet.swift` | 10 | 103, 117 |
| `Sources/SpeechPlayback.swift` | 0 | 35, 40, 79 |
| `Sources/StartNewChatIntent.swift` | 0 | 6, 7, 21, 23 |
| `Sources/SummarizeTextIntent.swift` | 0 | 6, 7, 21, 26, 39 |
| `Sources/VoiceDictation.swift` | 0 | 89, 98, 113, 126, 142, 163, 176, 186, 186, 188, 201, 226, 226, 235, 236, 254 |
| `Sources/WaveformView.swift` | 0 | — |

<details><summary>PocketAI.Iphone: the 264 sites in full</summary>

**`Sources/AppModel.swift`**

- L101 — `PocketAI's on-device engine couldn't start. Please restart the app; if it keeps happening, reinstall. (%arg)`

**`Sources/AskPocketAIIntent.swift`**

- L12 — `Ask PocketAI`
- L13 — `Ask PocketAI's on-device model a question.`
- L27 — `PocketAI couldn't generate a response: %arg`
- L44 — `PocketAI couldn't generate a response: %arg`

**`Sources/ChatView.swift`**

- L174 — `Choose a model`
- L176 — `Apple Intelligence`
- L357 — `New chat`
- L376 — `New chat`
- L419 — `Auto-speak replies disabled`
- L419 — `Auto-speak replies enabled`
- L455 — `Couldn't delete this chat: %arg`
- L525 — `Voice input`
- L526 — `PocketAI uses the microphone and on-device speech recognition to turn your voice into text. Audio is transcribed on this iPhone and never leaves it.`
- L527 — `Enable voice input`
- L818 — `\[(\d+)\]`
- L832 — `Project query`
- L923 — `/image`
- L932 — `Analyze the architectural trade-offs of deploying large language models on edge devices.`
- L1115 — `No model is ready yet. (%arg)`
- L1118 — `This needs PocketAI Pro. Unlock it in Settings.`
- L1120 — `Something went wrong: %arg`
- L1232 — `Recording hands-free voice input`
- L1232 — `Recording voice input`
- L1248 — `Insert voice text`
- L1248 — `Send voice message`
- L1249 — `Transcribes audio and places text in the composer`
- L1249 — `Transcribes audio and sends the message`
- L1344 — `attach-%arg.jpg`
- L1347 — `Couldn't read that image.`
- L1355 — `Couldn't attach the image: %arg`
- L1379 — `# %arg

`
- L1379 — `PocketAI chat`
- L1384 — `**%arg:**

%arg

`

**`Sources/ChatsListView.swift`**

- L84 — `Previous 7 Days`
- L85 — `Previous 30 Days`
- L373 — `New chat`
- L438 — `Summarize a long email`
- L439 — `Explain a concept simply`
- L440 — `Draft a quick reply`
- L582 — `No matching projects`
- L582 — `No projects yet`

**`Sources/DevicesView.swift`**

- L19 — `perm.primedPairing`
- L40 — `Find your Mac`
- L41 — `PocketAI uses the camera to scan your Mac's pairing code and Local Network access to connect to it. Everything stays on your local network.`
- L97 — `Open PocketAI on your Mac`
- L98 — `Go to Devices → Pair iPhone to show your QR code.`
- L102 — `Scan the pairing code`
- L103 — `Point this iPhone camera at your Mac screen.`
- L319 — `Browse models on this Mac…`
- L363 — `Enter a hostname or IP, optionally with :port — e.g. mac.tailnet.ts.net:%arg.`
- L436 — `your Mac`
- L522 — `Mac not reachable. Make sure both are on the same Wi-Fi, or enter the address.`
- L535 — `peer-%arg`
- L541 — `Pairing isn't available right now — the on-device engine didn't start. Restart the app and try again.`
- L560 — `Pairing was denied on your Mac.`
- L563 — `Mac not reachable. Make sure both are on the same Wi-Fi, or enter the address. (%arg)`
- L571 — `Pairing timed out. Make sure your Mac is awake and on the same Wi-Fi, then try again.`

**`Sources/DiagnosticsPreviewView.swift`**

- L204 — `support@pocket.liveshortly.com`
- L205 — `PocketAI Diagnostic Report`

**`Sources/DiagnosticsReport.swift`**

- L21 — `App lifecycle: process started`
- L153 — `\b(?:sk|pk|key)-[A-Za-z0-9_\-]{16,}\b`
- L160 — `\b(?:[A-Za-z0-9]{4,8}-){3,}[A-Za-z0-9]{4,8}\b`
- L167 — `\b[0-9a-fA-F]{24,}\b`
- L174 — `\b[A-Za-z0-9+/=_-]{32,}\b`

**`Sources/HomeView.swift`**

- L172 — `Chats on this iPhone`
- L174 — `Most used model`
- L175 — `No replies yet`
- L185 — `Downloaded models`
- L210 — `Show all (%arg)`
- L210 — `Show less`
- L223 — `Active downloads`
- L252 — `Recent chats`
- L252 — `See all`
- L265 — `home_recent_%arg`
- L425 — `Checking files…`
- L426 — `Paused · %arg of %arg`
- L464 — `Pause download`
- L464 — `Resume download`
- L493 — `Ready to chat`
- L494 — `No model selected`
- L505 — `New chat`
- L506 — `Choose model`
- L534 — `Choose a model to start chatting.`
- L642 — `Choose a model`
- L646 — `New chat`
- L666 — `Choose a model to start chatting.`
- L670 — `Apple Intelligence`
- L670 — `On this iPhone`
- L671 — `Apple Intelligence isn't ready on this device.`
- L678 — `That Mac isn't available right now.`
- L682 — `On this iPhone`
- L684 — `The selected model isn't installed. Choose another.`
- L690 — `Apple Intelligence`
- L690 — `On this iPhone`
- L701 — `On this iPhone`

**`Sources/ImageGenerationSheet.swift`**

- L70 — `No image model installed — install one in Models`
- L81 — `No image model installed — install one in Models`
- L123 — `No image model installed — install one in Models`
- L159 — `Failed to save: %arg`
- L191 — `gen-%arg.png`
- L469 — `Save to Photos`
- L479 — `Image saved to Photos`
- L479 — `Save image to Photos`
- L547 — `Light - Idle`
- L548 — `A neon cyberpunk street at night`
- L551 — `Dark - Idle`
- L552 — `A neon cyberpunk street at night`
- L557 — `A neon cyberpunk street at night`
- L563 — `A neon cyberpunk street at night`

**`Sources/ModelPickerView.swift`**

- L17 — `Default model`
- L18 — `Model for this chat`
- L24 — `Used for new chats. Existing chats keep their selected model.`
- L25 — `Changes the model for this conversation only. History is preserved.`
- L63 — `Apple Intelligence`
- L64 — `Built in · On this iPhone`

**`Sources/ModelsView.swift`**

- L19 — `On this iPhone`
- L21 — `On your Mac`
- L29 — `Smallest download`
- L58 — `models.userPausedIds`
- L226 — `models_filter_%arg`
- L399 — `model_action_button_%arg`
- L411 — `model_hero_%arg`
- L505 — `“%arg”`
- L506 — `Fits only`
- L507 — `“%arg”`
- L508 — `these filters`
- L519 — `Paused — Offline mode is on`
- L519 — `Waiting for network…`
- L524 — `Turn off Offline mode in Settings to resume downloads.`
- L525 — `Download paused — no connection. It resumes automatically when you're back online.`
- L564 — `model_row_%arg`
- L702 — `Download failed — check your connection, then retry.`
- L703 — `Download failed — not enough storage.`
- L704 — `Download failed — files didn't verify. Retry to download again.`
- L705 — `Download failed.`
- L730 — `On this iPhone`
- L760 — `Image generation`
- L881 — `No saved credential for %arg - try pairing again.`
- L898 — `Couldn't reach %arg: %arg`
- L902 — `Couldn't reach %arg: %arg`
- L911 — `Fits on Mac: %arg`
- L926 — `Use on phone`
- L927 — `Downloading…`
- L929 — `Checking files…`
- L931 — `Download on Mac`
- L978 — `Search failed: %arg`
- L1000 — `Set as default`
- L1003 — `Checking files…`
- L1006 — `Won't fit`
- L1048 — `paused, offline mode on`
- L1051 — `checking files`
- L1052 — `failed: %arg`
- L1181 — `Download failed (%arg).`
- L1181 — `Download failed — check your connection.`

**`Sources/OnboardingFlow.swift`**

- L145 — `Runs on this iPhone, even offline`
- L146 — `Inference executes locally via Metal and MLX.`
- L150 — `Nothing leaves your device`
- L151 — `See every network connection in the egress log.`
- L155 — `Use your Mac's bigger models`
- L156 — `Pair over local Wi-Fi to tap into your Mac's unified memory.`
- L224 — `Apple Intelligence available`
- L355 — `Download %arg`
- L487 — `Models stay completely on this device — nothing is uploaded.`
- L488 — `You can switch or download other models anytime in the Models tab.`
- L489 — `Pair your Mac anytime to run its larger models from your phone.`
- L535 — `Download failed — check your connection.`
- L536 — `Download failed (%arg).`
- L609 — `Summarize a long email`
- L610 — `Explain a concept simply`
- L611 — `Draft a quick reply`

**`Sources/PerfDiagnostics.swift`**

- L141 — `date,device,model,ttft_ms,decode_tps,peak_mem_mb,thermal
`

**`Sources/PermissionPrimer.swift`**

- L130 — `Find your Mac on Wi-Fi`
- L131 — `PocketAI uses Local Network access to discover and connect to your Mac. Nothing leaves your local network.`
- L142 — `Voice input`
- L143 — `PocketAI uses the microphone and on-device speech recognition to turn your voice into text. Audio is transcribed on this iPhone.`
- L144 — `Enable voice input`
- L155 — `Scan your Mac's code`
- L156 — `PocketAI uses the camera to scan the pairing QR code shown on your Mac.`
- L157 — `Open camera`

**`Sources/PocketAIApp.swift`**

- L30 — `PocketAIApp: application initialized`
- L49 — `
[REMOTE_SELFTEST_DONE] promptTokens=%arg, completionTokens=%arg
`
- L55 — `
[REMOTE_SELFTEST_ERROR] %arg
`
- L110 — `Qwen 2.5 0.5B`
- L118 — `Apache-2.0`
- L119 — `mlx-community/Qwen2.5-0.5B-Instruct-4bit`
- L134 — `Hello from iPhone! Say 'Remote model is working' and nothing else.`
- L151 — `
[REMOTE_SELFTEST_ERROR] %arg
`

**`Sources/PocketAIShortcuts.swift`**

- L8 — `Ask %arg`
- L9 — `Talk to %arg`
- L11 — `Ask PocketAI`
- L17 — `Summarize with %arg`
- L18 — `Summarize text with %arg`
- L20 — `Summarize Text`
- L26 — `New chat in %arg`
- L27 — `Start new chat in %arg`
- L29 — `New Chat`

**`Sources/ProjectDetailView.swift`**

- L311 — `New chat`
- L526 — `project.lsSessions.%arg`
- L563 — `Could not access the selected file.`

**`Sources/SettingsDestinations.swift`**

- L70 — `storage_row_%arg`
- L116 — `Runs MLX models on Apple silicon. The default on-device engine.`
- L123 — `Runs GGUF models on-device via Metal. Built into the core.`
- L129 — `Apple Intelligence`
- L130 — `Apple's on-device foundation model (iOS 26+ on supported devices).`
- L189 — `Server-based`
- L220 — `Not requested`
- L291 — `tts_voice_row_%arg`
- L309 — `The quick brown fox jumps over the lazy dog.`
- L326 — `Go to Devices → Pair iPhone to show your QR code.`
- L326 — `Open PocketAI on your Mac`
- L327 — `Open the Mac tab here`
- L327 — `Tap Mac in the tab bar, then Scan pairing code to point this iPhone at your Mac.`

**`Sources/SettingsView.swift`**

- L37 — `Default chat model`
- L44 — `Storage used`
- L51 — `Apple + MLX + llama.cpp`
- L51 — `Engine availability`
- L51 — `MLX + llama.cpp`
- L61 — `Speak replies`
- L67 — `Speech-to-text`
- L73 — `Text-to-speech voice`
- L83 — `Offline mode`
- L91 — `Network Proof`
- L98 — `Share diagnostic reports`
- L110 — `Send diagnostic report…`
- L136 — `Pair a Mac`
- L171 — `Not connected`
- L190 — `Apple Intelligence`
- L194 — `On your Mac`

**`Sources/SharedInbox.swift`**

- L164 — `Shared app group container is not available.`
- L166 — `Shared item payload file not found.`

**`Sources/SharedInboxIngestSheet.swift`**

- L49 — `Adding %arg of %arg: %arg...`
- L53 — `Could not locate payload for '%arg'`
- L68 — `Skipped '%arg': %arg`

**`Sources/SourceSheet.swift`**

- L103 — `Relevance score: %.2f`
- L117 — `Page %arg`

**`Sources/SpeechPlayback.swift`**

- L35 — `speech_%arg.wav`
- L40 — `Speaking reply`
- L79 — `Speech stopped`

**`Sources/StartNewChatIntent.swift`**

- L6 — `Start New Chat`
- L7 — `Create a new conversation in PocketAI.`
- L21 — `PocketAI couldn't create a new chat: %arg`
- L23 — `Started a new chat in PocketAI.`

**`Sources/SummarizeTextIntent.swift`**

- L6 — `Summarize Text`
- L7 — `Summarize a block of text using PocketAI.`
- L21 — `PocketAI couldn't generate a response: %arg`
- L26 — `Summarize the following text concisely:

%arg`
- L39 — `PocketAI couldn't generate a response: %arg`

**`Sources/VoiceDictation.swift`**

- L89 — `Speech recognition permission has been denied. Please enable it in Settings to transcribe voice input.`
- L98 — `Speech recognition permission is required for voice input.`
- L113 — `Microphone permission has been denied. Please enable it in Settings to record audio.`
- L126 — `Microphone permission has been denied. Please enable it in Settings to record audio.`
- L142 — `Microphone permission is required for voice input.`
- L163 — `dictation_%arg.m4a`
- L176 — `Failed to start recording audio.`
- L186 — `Hands-free voice recording started`
- L186 — `Voice recording started`
- L188 — `Failed to initialize recording: %arg`
- L201 — `Transcribing voice input`
- L226 — `Voice message ready to send`
- L226 — `Voice transcribed`
- L235 — `Transcription failed: %arg`
- L236 — `Transcription failed`
- L254 — `Recording cancelled`

</details>

### PocketAI.Mac

236 string sites still need a code change; 260 literals already sit in a `LocalizedStringKey` / `LocalizedStringResource` position and resolve from `Bundle.main` as soon as the key exists in the catalog (these are already seeded in English).

| File | Already localizable | Sites to convert (line numbers) |
|---|---:|---|
| `Sources/AllListsView.swift` | 14 | 77, 113, 113, 181, 183, 189, 189, 250, 271, 271, 317, 332 |
| `Sources/AppModel.swift` | 0 | 92, 134, 159, 175, 222, 236, 349 |
| `Sources/ChatModel.swift` | 0 | 48, 92, 199 |
| `Sources/ChatView.swift` | 14 | 183, 183, 376, 391, 393, 395, 424, 426, 435, 438, 442, 446, 452, 482, 483, 484 |
| `Sources/CommandPaletteView.swift` | 2 | 61, 108, 109, 119, 129, 130, 131, 143, 144, 147, 148, 151, 152, 156, 159, 165, 171, 172 |
| `Sources/ComposerTextView.swift` | 0 | 15 |
| `Sources/DevicesView.swift` | 11 | 25, 25, 46, 46, 52, 100, 139, 150, 204, 214 |
| `Sources/Dictation.swift` | 0 | — |
| `Sources/HUD.swift` | 2 | 189 |
| `Sources/ImageGenView.swift` | 11 | 64, 82, 93, 100, 173, 198, 213, 234, 240 |
| `Sources/LSChatView.swift` | 17 | 144, 219, 285, 285, 289, 290, 309, 329, 343, 352, 353, 416, 439, 503, 518, 548, 550, 552, 554, 557, 557, 560, 560 |
| `Sources/LSNewSessionView.swift` | 14 | 27, 68, 68, 159, 160, 225, 245, 246, 246, 247, 256 |
| `Sources/LSPermissionBar.swift` | 8 | 20, 90, 90, 192, 198 |
| `Sources/LSSessionModel.swift` | 0 | 41, 134, 172, 211, 278, 285, 296, 328 |
| `Sources/LSShareSheet.swift` | 13 | 72, 104, 105, 105, 106, 107, 169, 177, 178, 193, 269, 290, 308, 323 |
| `Sources/MessageRow.swift` | 3 | 88, 101, 158, 162, 205, 208, 209, 212, 214, 215, 216, 220 |
| `Sources/ModelsView.swift` | 16 | 39, 40, 47, 119, 242, 283, 291, 292, 293, 294, 358, 384, 386, 409, 412, 416, 418, 485 |
| `Sources/NewChatMenu.swift` | 8 | — |
| `Sources/OnboardingView.swift` | 12 | 47, 48, 49, 89, 90, 91, 95, 201, 202, 203 |
| `Sources/PocketAIApp.swift` | 12 | — |
| `Sources/PocketAIIntents.swift` | 0 | 15, 16, 31, 32, 40, 48, 49, 63, 72, 85, 89, 99, 99, 100, 104, 109, 110 |
| `Sources/ProjectDetailView.swift` | 15 | 130, 182, 199, 225, 251, 315, 348 |
| `Sources/ProjectSessions.swift` | 0 | 14 |
| `Sources/QuickAskPanel.swift` | 3 | 79, 109 |
| `Sources/ReadAloud.swift` | 0 | 65 |
| `Sources/RootView.swift` | 3 | — |
| `Sources/ScrollActivity.swift` | 0 | — |
| `Sources/ServerView.swift` | 12 | 69, 70, 109, 110, 121, 122, 130, 132, 218, 220, 246, 248 |
| `Sources/SettingsView.swift` | 30 | 103, 104, 167, 171, 214, 217, 218, 219, 219, 264, 264, 266 |
| `Sources/Sheets.swift` | 13 | 71 |
| `Sources/SidebarView.swift` | 27 | 155, 155, 344, 362, 414 |

<details><summary>PocketAI.Mac: the 236 sites in full</summary>

**`Sources/AllListsView.swift`**

- L77 — `Untitled chat`
- L113 — `No chats match “%arg”`
- L113 — `No chats yet`
- L181 — `Previous 7 days`
- L183 — `Previous 30 days`
- L189 — `Previous 30 days`
- L189 — `Previous 7 days`
- L250 — `Untitled session`
- L271 — `No sessions match “%arg”`
- L271 — `No sessions yet`
- L317 — `ended %arg`
- L332 — `Couldn't load sessions: %arg`

**`Sources/AppModel.swift`**

- L92 — `Qwen3 4B`
- L134 — `mlx-community/Qwen3-4B-Instruct-2507-4bit@50d427756c6b1b2fe0c0a10f67fbda1fc8e82c1b`
- L159 — `PocketAI's on-device engine couldn't start. Please restart the app; if it keeps happening, reinstall. (%arg)`
- L175 — `PocketAI: saved license re-activation failed (retrying next launch): %@`
- L222 — `Apple Intelligence`
- L236 — `Apple Intelligence`
- L349 — `Serving models`

**`Sources/ChatModel.swift`**

- L48 — `New chat`
- L92 — `No model is installed yet — open the model library and download one.`
- L199 — `Give this conversation a title of at most six words. Reply with the title alone — no quotes, no punctuation at the end, no preamble.

Message: %arg
Reply: %arg`

**`Sources/ChatView.swift`**

- L183 — `Model for this chat`
- L183 — `Stop generating to switch models`
- L376 — `Attach an image`
- L391 — `Stop dictation`
- L393 — `mic access denied`
- L395 — `⏎ send · ⇧⏎ newline`
- L424 — `This model can't read images. Switch to %arg to attach one.`
- L426 — `This model can't read images. Install a vision model to attach one.`
- L435 — `.md`
- L438 — `# %arg`
- L442 — `## %arg · %arg`
- L446 — `![attachment](%arg)`
- L452 — `Couldn't save the transcript: %arg`
- L482 — `Explain this error message`
- L483 — `Summarise the text I paste next`
- L484 — `Draft a commit message for these changes`

**`Sources/CommandPaletteView.swift`**

- L61 — `↑↓ move · ⏎ open · esc close`
- L108 — `chat-%arg`
- L109 — `Untitled chat`
- L119 — `project-%arg`
- L129 — `session-%arg`
- L130 — `Untitled session`
- L131 — `LiveShortly · live`
- L143 — `New chat`
- L144 — `⌘N`
- L147 — `Model library`
- L148 — `Download and switch models`
- L151 — `Generate an image`
- L152 — `On-device SDXL Turbo`
- L156 — `Pair an iPhone`
- L159 — `Local server`
- L165 — `Offline mode and logs`
- L171 — `New LiveShortly session`
- L172 — `⇧⌘N`

**`Sources/ComposerTextView.swift`**

- L15 — `Message…`

**`Sources/DevicesView.swift`**

- L25 — `Pair a device…`
- L25 — `Start server & pair…`
- L46 — `Devices using this Mac`
- L46 — `None paired yet`
- L52 — `Macs this app can borrow`
- L100 — `Scan on the iPhone`
- L139 — `seen %arg`
- L150 — `:%arg`
- L204 — `The local server didn't start, so there is nothing for a device to pair with. Open Local server and start it there to see why.`
- L214 — `Couldn't create a pairing code: %arg`

**`Sources/HUD.swift`**

- L189 — `just now`

**`Sources/ImageGenView.swift`**

- L64 — `Avoid · optional`
- L82 — `Steps · %arg`
- L93 — `Seed · optional`
- L100 — `Generating…`
- L173 — `Loading the image model…`
- L198 — `Step %arg of %arg`
- L213 — `Generation failed: %arg`
- L234 — `Couldn't encode the image as PNG.`
- L240 — `Couldn't save: %arg`

**`Sources/LSChatView.swift`**

- L144 — `ON FEED`
- L219 — `◆ assistant`
- L285 — `No activity was recorded`
- L285 — `Session is live`
- L289 — `Say something below, or watch the agent start up.`
- L290 — `This session ended without producing any events.`
- L309 — `Message this session…`
- L329 — `Stop dictation`
- L343 — `⏎ send · ⇧⏎ newline · goes to the agent as a viewer message`
- L352 — `Read only — you can watch but not comment.`
- L353 — `Session ended — read only.`
- L416 — `◆ agent`
- L439 — `awaiting %arg`
- L503 — `agent activity · %arg step%arg`
- L518 — `… %arg earlier steps`
- L548 — `› %arg %arg`
- L550 — `› %arg`
- L552 — `› %arg %arg`
- L554 — `› %arg %arg`
- L557 — `  %arg`
- L557 — `› output`
- L560 — `› %arg`
- L560 — `› %arg`

**`Sources/LSNewSessionView.swift`**

- L27 — ` · offline`
- L68 — `New agent session`
- L68 — `New agent session in this project`
- L159 — `This machine hasn't checked in recently. Starting a session `
- L160 — `may produce one with no agent attached.`
- L225 — `Couldn't load your machines: %arg`
- L245 — `The session was created, but the machine refused to start `
- L246 — `the agent: %arg. `
- L246 — `unknown error`
- L247 — `It is in the sidebar if you still want to open it.`
- L256 — `Couldn't start the session: %arg`

**`Sources/LSPermissionBar.swift`**

- L20 — `permission requested`
- L90 — `Input request`
- L90 — `Permission request`
- L192 — `… %arg more removed`
- L198 — `… %arg more added`

**`Sources/LSSessionModel.swift`**

- L41 — `Untitled session`
- L134 — `Stream dropped — reopen the session to catch up.`
- L172 — `local-%arg`
- L211 — `Couldn't send that answer: %arg`
- L278 — `d%arg`
- L285 — `e%arg`
- L296 — `w%arg`
- L328 — `@%arg`

**`Sources/LSShareSheet.swift`**

- L72 — `Who can open it`
- L104 — `Only you and people you invite below.`
- L105 — `Anyone with the link`
- L105 — `Not listed anywhere; the link is the key.`
- L106 — `Anyone with the link, and it can appear on the feed.`
- L107 — `Readable without signing in at all.`
- L169 — `Public feed`
- L177 — `Listed on the LiveShortly feed where anyone can find it.`
- L178 — `Publishing lists this session on the public feed for anyone to find.`
- L193 — `Invited people · %arg`
- L269 — `Couldn't change access: %arg`
- L290 — `Couldn't update the feed: %arg`
- L308 — `Couldn't invite %arg: %arg`
- L323 — `Couldn't remove access: %arg`

**`Sources/MessageRow.swift`**

- L88 — `◆ assistant`
- L101 — `read aloud`
- L158 — `◆ assistant`
- L162 — `thinking…`
- L205 — `No model is installed yet. Open the model library and download one.`
- L208 — `This Mac ran out of memory for that model. Try a smaller one, `
- L209 — `or unload other models first.`
- L212 — `That reply was stopped.`
- L214 — `too long`
- L215 — `The conversation is longer than this model's context. `
- L216 — `Start a new chat to continue.`
- L220 — `Couldn't reach the network. Local models still work offline.`

**`Sources/ModelsView.swift`**

- L39 — `No models installed yet. Turn off “installed only” to browse the catalog.`
- L40 — `No models match “%arg”.`
- L47 — `Hugging Face`
- L119 — `On disk`
- L242 — `Hugging Face search failed: %arg`
- L283 — `Core ML`
- L291 — `Runs on Apple's MLX runtime`
- L292 — `Runs on the built-in llama.cpp engine`
- L293 — `Runs on Core ML`
- L294 — `Speech model, runs on WhisperKit`
- L358 — `verifying…`
- L384 — `license unknown`
- L386 — `NON-COMMERCIAL`
- L409 — `The downloaded file didn't match its checksum. Retry to fetch it again.`
- L412 — `Not enough disk space for this model.`
- L416 — `The download lost its connection.`
- L418 — `The download failed.`
- L485 — `Get anyway`

**`Sources/OnboardingView.swift`**

- L47 — `No account, no cloud — your chats never leave this machine.`
- L48 — `Models run on Apple silicon through MLX.`
- L49 — `Pair your iPhone later and it can use this Mac's models.`
- L89 — `⌘N starts a chat, ⌘K jumps to anything.`
- L90 — `The menu bar icon asks from any app.`
- L91 — `Connect LiveShortly in Settings to watch your coding agents.`
- L95 — `Try one`
- L201 — `Explain what this app can do, briefly.`
- L202 — `Summarise the text I paste next.`
- L203 — `Draft a commit message from a diff I'll paste.`

**`Sources/PocketAIIntents.swift`**

- L15 — `Ask PocketAI`
- L16 — `Ask PocketAI's on-device model a question.`
- L31 — `Summarize with PocketAI`
- L32 — `Summarize text on this Mac, privately.`
- L40 — `Summarise the following text in a few sentences:

%arg`
- L48 — `Start a PocketAI chat`
- L49 — `Opens PocketAI with a new chat ready.`
- L63 — `PocketAI isn't running yet. Open it once and try again.`
- L72 — `PocketAI couldn't start a conversation: %arg`
- L85 — `PocketAI couldn't generate a response: %arg`
- L89 — `PocketAI returned an empty response.`
- L99 — `Ask %arg`
- L99 — `Ask %arg a question`
- L100 — `Ask PocketAI`
- L104 — `Summarize with %arg`
- L109 — `New %arg chat`
- L110 — `New chat`

**`Sources/ProjectDetailView.swift`**

- L130 — `Documents · %arg`
- L182 — `Chats in this project · %arg`
- L199 — `Untitled chat`
- L225 — `LiveShortly sessions · %arg`
- L251 — `Untitled session`
- L315 — `Kept for this window only — the core has no call to store project instructions yet, so this does not survive a relaunch.`
- L348 — `Couldn't add %arg: %arg`

**`Sources/ProjectSessions.swift`**

- L14 — `project.lsSessions.%arg`

**`Sources/QuickAskPanel.swift`**

- L79 — `⏎ ask`
- L109 — `Couldn't start a conversation.`

**`Sources/ReadAloud.swift`**

- L65 — `en-US`

**`Sources/ServerView.swift`**

- L69 — `Port %arg · %arg connected`
- L70 — `Other apps and paired devices can't reach this Mac's models.`
- L109 — `Reachable from your LAN, so a paired iPhone can use this Mac. macOS will ask for Local Network permission.`
- L110 — `Loopback only (127.0.0.1) — nothing outside this Mac can connect.`
- L121 — `For other apps`
- L122 — `Base URL`
- L130 — `not generated`
- L132 — `Generate & copy`
- L218 — `Network activity · %arg`
- L220 — `Nothing has left this Mac.`
- L246 — `Audit log · %arg`
- L248 — `No recorded actions yet.`

**`Sources/SettingsView.swift`**

- L103 — `macOS refused to change this: %arg. `
- L104 — `A signed build is usually required.`
- L167 — `This is how replies will sound.`
- L171 — `Read aloud`
- L214 — `PocketAI for Mac`
- L217 — `Apple silicon, .safetensors models`
- L218 — `GGUF models, built in`
- L219 — `Apple Intelligence`
- L219 — `System model, macOS 15.1+ on supported Macs`
- L264 — `Pro activated on this Mac.`
- L264 — `That key did not unlock Pro.`
- L266 — `Activation failed: %arg`

**`Sources/Sheets.swift`**

- L71 — `Instructions · optional`

**`Sources/SidebarView.swift`**

- L155 — `No chats yet`
- L155 — `No matches`
- L344 — `Untitled chat`
- L362 — `Untitled session`
- L414 — `pocket.conversationsChanged`

</details>
