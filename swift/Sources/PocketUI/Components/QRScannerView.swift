import SwiftUI
#if os(iOS) && !targetEnvironment(simulator)
import AVFoundation
#endif

/// C15: full-bleed camera QR scanner with a rounded viewfinder overlay and
/// haptic on detect. Real `AVCaptureSession` + `AVCaptureMetadataOutput`
/// (`.qr`) on a device; the Simulator has no camera at all, so that path
/// shows a placeholder instead of a black/frozen preview layer.
public struct QRScannerView: View {
    let onCode: (String) -> Void

    public init(onCode: @escaping (String) -> Void) {
        self.onCode = onCode
    }

    public var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            #if os(iOS) && !targetEnvironment(simulator)
            CameraPreview(onCode: onCode)
                .ignoresSafeArea()
            #else
            VStack(spacing: PocketSpacing.s12) {
                Image(systemName: "camera.fill").font(.system(size: 40)).foregroundStyle(.white.opacity(0.6))
                Text("Camera preview isn't available in Simulator")
                    .font(.pocket(.footnote))
                    .foregroundStyle(.white.opacity(0.6))
            }
            #endif

            RoundedRectangle(cornerRadius: 24)
                .stroke(PocketColor.accent, lineWidth: 3)
                .frame(width: 240, height: 240)
        }
    }
}

#if os(iOS) && !targetEnvironment(simulator)
private struct CameraPreview: UIViewControllerRepresentable {
    let onCode: (String) -> Void

    func makeUIViewController(context: Context) -> ScannerViewController {
        let controller = ScannerViewController()
        controller.onCode = onCode
        return controller
    }

    func updateUIViewController(_ uiViewController: ScannerViewController, context: Context) {}
}

final class ScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var onCode: ((String) -> Void)?
    private let session = AVCaptureSession()
    private var didDetect = false

    override func viewDidLoad() {
        super.viewDidLoad()
        guard let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device),
              session.canAddInput(input) else { return }
        session.addInput(input)

        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else { return }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        output.metadataObjectTypes = [.qr]

        let preview = AVCaptureVideoPreviewLayer(session: session)
        preview.videoGravity = .resizeAspectFill
        preview.frame = view.bounds
        view.layer.addSublayer(preview)

        DispatchQueue.global(qos: .userInitiated).async { [session] in
            session.startRunning()
        }
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        view.layer.sublayers?.first?.frame = view.bounds
    }

    func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput metadataObjects: [AVMetadataObject], from connection: AVCaptureConnection) {
        guard !didDetect,
              let object = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
              let value = object.stringValue else { return }
        didDetect = true
        session.stopRunning()
        UINotificationFeedbackGenerator().notificationOccurred(.success)
        onCode?(value)
    }
}
#endif
