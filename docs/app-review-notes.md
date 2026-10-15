# App Store Review Notes — PocketAI

**Application Name:** PocketAI  
**Bundle Identifier:** `app.getpocketai.pocketai`  
**Platforms:** macOS (macOS 15+) & iOS (Universal Purchase)  
**In-App Purchase Product ID:** `com.getpocketai.app.pro` (also referenced in StoreKit as `app.getpocketai.pro`)  
**Demo Video Link:** `<DEMO_VIDEO_URL>` *(≤ 2 min demonstration: install → model download → offline chat → LAN pairing → remote inference)*  
**Reviewer Test License Key:** `<REVIEWER_TEST_KEY>`  

---

## 1. Quick Testing Instructions for App Reviewers

PocketAI executes machine learning models locally on Apple Silicon hardware using bundled inference runtimes (Apple MLX, llama.cpp, CoreML). Release builds execute real local inference without mock preview modes.

To test the application quickly without large downloads:

1. **Launch & Onboarding**:
   - Open PocketAI. Proceed through the brief onboarding screen.
2. **Download Review-Friendly Model**:
   - In the model catalog / model picker, select **Qwen3-0.6B-4bit** (tagged `review_friendly`).
   - This model is specifically designated for review testing: download size is only **~347 MB** (total runtime footprint **~399 MB**), downloading in seconds on typical broadband.
3. **Exercise On-Device Chat**:
   - Type a prompt in the composer (e.g. *"Explain what makes on-device AI private in two sentences"*).
   - Observe immediate token generation streamed directly from the local inference engine.
   - Note the **Network Proof** indicator confirming zero internet egress during inference.
4. **Test Pro Features & License Activation**:
   - Open **Settings** → **Pro** (or click the Pro upgrade prompt).
   - Click **"Have a license key?"** and enter the Reviewer Test Key:
     ```
     <REVIEWER_TEST_KEY>
     ```
     *(Alternatively, open the activation URL: `pocketai://activate?k=<REVIEWER_TEST_KEY>`)*.
   - Pro status activates immediately via offline cryptographic verification.
5. **Test Local Server & Device Pairing (Pro Feature)**:
   - In Settings, open the **Server** tab and enable the local server.
   - PocketAI exposes an OpenAI-compatible HTTP/SSE API endpoint on loopback (`http://127.0.0.1:<PORT>`) and, when explicitly enabled by the user, across the local area network (LAN) for paired personal devices.

---

## 2. Entitlement & Networking Justification

### `com.apple.developer.networking` & `com.apple.security.network.server`

* **Purpose:**
  PocketAI includes a local server allowing the user to share their Mac's compute and local models with their **own paired personal devices** (e.g., querying Mac-hosted models from their paired iPhone or personal scripts over the local Wi-Fi network).
* **Off by Default:**
  LAN server functionality is disabled by default. When the user enables the server, it binds strictly to loopback (`127.0.0.1`) until the user explicitly toggles LAN sharing.
* **Private & Authenticated:**
  The server is **never a public internet server**. LAN connections require mutual pairing via local QR code or PIN exchange, authenticated using device-generated TLS certificates and Ed25519 signatures. Bonjour (`_pocketai._tcp`) is advertised only when local LAN serving is active.
* **Info.plist Strings:**
  `NSLocalNetworkUsageDescription` clearly discloses this purpose: *"PocketAI lets your paired iPhone use this Mac's models."*

---

## 3. Model Weights Are User Data, Not Executable Code (Guideline 2.5.2)

PocketAI complies strictly with App Store Review Guideline 2.5.2 regarding remote executable code:

* **Weights are Data Files:**
  The downloaded files (`model.safetensors`, `tokenizer.json`, `.gguf`) are purely static numeric tensor weight data and tokenization tables. They contain **zero executable code, zero compiled binaries, and zero dynamic scripts**.
* **Bundled Sandboxed Engines:**
  Inference is executed exclusively by pre-compiled native engines bundled into the sandboxed application binary at build time (Apple MLX framework, llama.cpp, CoreML).
* **Provenance & Integrity:**
  Model files are downloaded directly over HTTPS from public repositories on Hugging Face (such as `mlx-community/Qwen3-0.6B-4bit`). Every file is pinned to an immutable 40-character Git commit hash with explicit SHA-256 verification and file size checks before being accepted by the app.
* **No Dynamic Execution:**
  The app contains no JIT compiler, no dynamic code loader, and does not alter executable behavior at runtime.

---

## 4. In-App Purchase & External License Key Compliance (Guidelines 3.1.1 & 3.1.3(b))

PocketAI honors both in-app purchases and website-purchased license keys in full compliance with Apple Review Guidelines:

* **Guideline 3.1.1 (In-App Purchase):**
  PocketAI Pro is offered as a non-consumable In-App Purchase (`com.getpocketai.app.pro` / `app.getpocketai.pro`) directly within the app using StoreKit 2. Customers can purchase, restore, and manage this unlock entirely through Apple In-App Purchase. Universal Purchase shares the purchase across macOS and iOS.
* **Guideline 3.1.3(b) (Multiplatform Services):**
  PocketAI also allows users who have purchased a license on our website to access Pro features by entering an Ed25519-signed license key (issued via Keygen and verified 100% offline). This satisfies Guideline 3.1.3(b) because **the exact same unlock is available as an in-app purchase** within the app.
* **No Steering:**
  The app contains no external purchase links, no website pricing comparisons, and no calls-to-action directing users away from Apple's In-App Purchase system.
* **Reviewer Access:**
  App Reviewers may use the test key above (`<REVIEWER_TEST_KEY>`) to immediately unlock Pro capabilities for testing, or test the in-app purchase flow using the StoreKit sandbox environment.

---

## 5. Privacy Architecture & App Privacy Nutrition Label

* **App Privacy Label:**
  **"Data Not Collected"**. PocketAI does not collect, track, or transmit any user data, personal information, prompts, or generated responses.
* **100% On-Device:**
  All text, image, audio, and embeddings processing runs entirely locally on the user's Apple Silicon GPU/ANE/CPU.
* **Network Proof:**
  The app provides a real-time Network Proof inspection panel and local cryptographic audit log, verifying that zero network egress occurs while models are running.
* **Full Offline Operation:**
  The application functions with zero internet connection (airplane mode) once model weights are stored locally.

---

## 6. Safety & Content Safeguards (Guideline 4.7)

* **Curated Catalog:**
  The default catalog contains only curated models from reputable organizations (such as Alibaba Qwen, Google Gemma, Meta Llama) with known safety alignments. No uncensored models are included in the curated catalog.
* **Report-a-Response Action:**
  Users can tap/click "Report a response" on any generated message to capture problematic outputs. Reports are saved locally and can optionally be attached to an email to support if the user chooses.
* **Export Compliance:**
  Encryption in PocketAI is limited to standard Apple platform encryption, HTTPS/TLS for model downloads, and Ed25519 signature checks, qualifying for standard export compliance exemptions.

---

## Summary of Reviewer Resources

| Resource | Value |
|---|---|
| **Fast Test Model** | `mlx-community/Qwen3-0.6B-4bit` (~347 MB download, ~399 MB runtime) |
| **Catalog Tag** | `review_friendly` |
| **Demo Video Link** | `<DEMO_VIDEO_URL>` |
| **Reviewer Pro Test Key** | `<REVIEWER_TEST_KEY>` |
| **Pro IAP Product ID** | `com.getpocketai.app.pro` (StoreKit) |
| **Support Contact** | `<SUPPORT_EMAIL>` |
