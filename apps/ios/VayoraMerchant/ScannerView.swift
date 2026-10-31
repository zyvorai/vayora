import AVFoundation
import SwiftUI

/// Camera QR scanner. Calls `onCode` once with the first code read.
struct ScannerView: View {
    var onCode: (String) -> Void
    var onCancel: () -> Void
    @State private var denied = false

    var body: some View {
        ZStack(alignment: .topTrailing) {
            CameraPreview(onCode: onCode, onDenied: { denied = true }).ignoresSafeArea()
            RoundedRectangle(cornerRadius: 24).stroke(.white, lineWidth: 3).frame(width: 250, height: 250)
                .frame(maxWidth: .infinity, maxHeight: .infinity).allowsHitTesting(false)
            Button { onCancel() } label: {
                Image(systemName: "xmark.circle.fill").font(.largeTitle).symbolRenderingMode(.palette).foregroundStyle(.white, .black.opacity(0.4))
            }.padding().accessibilityLabel("Close scanner")
            if denied {
                Text("Camera access is off. Allow it in Settings, or type the address and key by hand.")
                    .padding().background(.thinMaterial, in: RoundedRectangle(cornerRadius: 12))
                    .frame(maxWidth: .infinity, maxHeight: .infinity).padding()
            }
        }
        .background(.black)
    }
}

struct CameraPreview: UIViewRepresentable {
    var onCode: (String) -> Void
    var onDenied: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(onCode) }

    func makeUIView(context: Context) -> PreviewView {
        let v = PreviewView()
        let c = context.coordinator
        AVCaptureDevice.requestAccess(for: .video) { ok in
            DispatchQueue.main.async {
                guard ok, let dev = AVCaptureDevice.default(for: .video),
                      let input = try? AVCaptureDeviceInput(device: dev), c.session.canAddInput(input) else { onDenied(); return }
                c.session.addInput(input)
                let out = AVCaptureMetadataOutput()
                if c.session.canAddOutput(out) {
                    c.session.addOutput(out)
                    out.setMetadataObjectsDelegate(c, queue: .main)
                    out.metadataObjectTypes = [.qr]
                }
                v.previewLayer.session = c.session
                v.previewLayer.videoGravity = .resizeAspectFill
                DispatchQueue.global().async { c.session.startRunning() }
            }
        }
        return v
    }
    func updateUIView(_ uiView: PreviewView, context: Context) {}
    static func dismantleUIView(_ uiView: PreviewView, coordinator: Coordinator) { coordinator.session.stopRunning() }

    final class PreviewView: UIView {
        override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
        var previewLayer: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }
    }

    final class Coordinator: NSObject, AVCaptureMetadataOutputObjectsDelegate {
        let session = AVCaptureSession()
        let onCode: (String) -> Void
        private var fired = false
        init(_ onCode: @escaping (String) -> Void) { self.onCode = onCode }
        func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objs: [AVMetadataObject], from: AVCaptureConnection) {
            guard !fired, let s = (objs.first as? AVMetadataMachineReadableCodeObject)?.stringValue else { return }
            fired = true
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            onCode(s)
        }
    }
}
