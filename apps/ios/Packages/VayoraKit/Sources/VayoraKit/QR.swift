import CoreImage.CIFilterBuiltins
import SwiftUI

public enum QR {
    /// A crisp QR for `text`, or nil when it cannot be encoded.
    public static func image(_ text: String, scale: CGFloat = 10) -> Image? {
        let f = CIFilter.qrCodeGenerator()
        f.message = Data(text.utf8)
        f.correctionLevel = "M"
        guard let out = f.outputImage?.transformed(by: CGAffineTransform(scaleX: scale, y: scale)),
              let cg = CIContext().createCGImage(out, from: out.extent) else { return nil }
        return Image(decorative: cg, scale: 1).interpolation(.none)
    }
}
