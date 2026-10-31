#if canImport(UIKit)
import AVFoundation
import SwiftUI
import UIKit

/// Camera QR scanner that locks on: corner brackets breathe while searching, then snap onto the code, turn
/// green, tap, and only then hand the text over. Shared by both apps.
public struct ScannerView: View {
    let brand: Brand, hint: String, onCode: (String) -> Void, onCancel: () -> Void
    @State private var denied = false
    @State private var lock: CGRect?
    @State private var torch = false
    @State private var breathe = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(brand: Brand = .pay, hint: String = "Aim at a QR code", onCode: @escaping (String) -> Void, onCancel: @escaping () -> Void) {
        self.brand = brand; self.hint = hint; self.onCode = onCode; self.onCancel = onCancel
    }

    public var body: some View {
        GeometryReader { g in
            let side: CGFloat = min(g.size.width * 0.68, 290)
            let idle = CGRect(x: (g.size.width - side) / 2, y: (g.size.height - side) / 2 - 30, width: side, height: side)
            let frame = lock?.insetBy(dx: -14, dy: -14) ?? idle
            ZStack {
                CameraPreview(onLock: { rect in lock = rect; Haptics.success() }, onCode: onCode, onDenied: { denied = true }, torch: torch)
                // dim everything except the viewfinder
                Path { p in p.addRect(CGRect(origin: .zero, size: g.size)); p.addRoundedRect(in: frame, cornerSize: CGSize(width: 26, height: 26)) }
                    .fill(.black.opacity(0.55), style: FillStyle(eoFill: true)).allowsHitTesting(false)
                    .animation(VUI.spring, value: lock)
                CornerBrackets().stroke(lock == nil ? Color.white : VUI.mint, style: StrokeStyle(lineWidth: 5, lineCap: .round, lineJoin: .round))
                    .frame(width: frame.width, height: frame.height).position(x: frame.midX, y: frame.midY)
                    .scaleEffect(lock == nil && breathe && !reduceMotion ? 1.035 : 1)
                    .animation(VUI.spring, value: lock)
                    .animation(.easeInOut(duration: 1.1).repeatForever(autoreverses: true), value: breathe)
                    .allowsHitTesting(false)
                VStack {
                    HStack {
                        Button(action: onCancel) {
                            Image(systemName: "xmark").font(.system(size: 16, weight: .bold)).foregroundStyle(.white).frame(width: 40, height: 40).background(.black.opacity(0.4), in: Circle())
                        }.accessibilityLabel("Close scanner")
                        Spacer()
                        Button { torch.toggle(); Haptics.tap() } label: {
                            Image(systemName: torch ? "flashlight.on.fill" : "flashlight.off.fill").font(.system(size: 16, weight: .bold)).foregroundStyle(torch ? VUI.amber : .white)
                                .frame(width: 40, height: 40).background(.black.opacity(0.4), in: Circle())
                        }.accessibilityLabel(torch ? "Turn the torch off" : "Turn the torch on")
                    }
                    .padding(.horizontal, 20).padding(.top, 8)
                    Spacer()
                    Text(lock == nil ? hint : "Got it").vfont(17, .semibold).foregroundStyle(.white)
                        .padding(.horizontal, 18).padding(.vertical, 10).background(.black.opacity(0.45), in: Capsule()).padding(.bottom, 56)
                }
                if denied {
                    Text("Camera access is off. Allow it in iPhone Settings, then try again.").vfont(15).foregroundStyle(.white)
                        .multilineTextAlignment(.center).padding(20).background(.black.opacity(0.7), in: RoundedRectangle(cornerRadius: 18)).padding(30)
                }
            }
            .background(.black)
        }
        .ignoresSafeArea()
        .onAppear { breathe = true }
    }
}

struct CornerBrackets: Shape {
    func path(in r: CGRect) -> Path {
        let l = min(38, min(r.width, r.height) * 0.22)
        var p = Path()
        for (x, y, dx, dy) in [(r.minX, r.minY, 1.0, 1.0), (r.maxX, r.minY, -1.0, 1.0), (r.minX, r.maxY, 1.0, -1.0), (r.maxX, r.maxY, -1.0, -1.0)] {
            p.move(to: CGPoint(x: x + dx * l, y: y)); p.addLine(to: CGPoint(x: x, y: y)); p.addLine(to: CGPoint(x: x, y: y + dy * l))
        }
        return p
    }
}

struct CameraPreview: UIViewRepresentable {
    var onLock: (CGRect) -> Void
    var onCode: (String) -> Void
    var onDenied: () -> Void
    var torch: Bool

    func makeCoordinator() -> Coordinator { Coordinator(onLock: onLock, onCode: onCode) }

    func makeUIView(context: Context) -> PreviewView {
        let v = PreviewView()
        let c = context.coordinator
        c.layer = v.previewLayer
        AVCaptureDevice.requestAccess(for: .video) { ok in
            DispatchQueue.main.async {
                guard ok, let dev = AVCaptureDevice.default(for: .video),
                      let input = try? AVCaptureDeviceInput(device: dev), c.session.canAddInput(input) else { onDenied(); return }
                c.session.addInput(input); c.device = dev
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
    func updateUIView(_ uiView: PreviewView, context: Context) { context.coordinator.setTorch(torch) }
    static func dismantleUIView(_ uiView: PreviewView, coordinator: Coordinator) { coordinator.setTorch(false); coordinator.session.stopRunning() }

    final class PreviewView: UIView {
        override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
        var previewLayer: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }
    }

    final class Coordinator: NSObject, AVCaptureMetadataOutputObjectsDelegate {
        let session = AVCaptureSession()
        var layer: AVCaptureVideoPreviewLayer?
        var device: AVCaptureDevice?
        let onLock: (CGRect) -> Void, onCode: (String) -> Void
        private var fired = false
        init(onLock: @escaping (CGRect) -> Void, onCode: @escaping (String) -> Void) { self.onLock = onLock; self.onCode = onCode }

        func setTorch(_ on: Bool) {
            guard let d = device, d.hasTorch, (try? d.lockForConfiguration()) != nil else { return }
            d.torchMode = on ? .on : .off; d.unlockForConfiguration()
        }

        func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objs: [AVMetadataObject], from: AVCaptureConnection) {
            guard !fired, let obj = objs.first as? AVMetadataMachineReadableCodeObject, let s = obj.stringValue else { return }
            fired = true
            if let l = layer, let t = l.transformedMetadataObject(for: obj) { onLock(t.bounds) }
            // let the brackets land on the code before the next screen takes over
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.45) { [onCode] in onCode(s) }
        }
    }
}
#endif
