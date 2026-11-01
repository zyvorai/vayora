import PhotosUI
import SwiftUI
import UIKit
import VayoraKit
import VayoraUI

/// Folder for receipt photos, inside the app's own documents.
enum ReceiptFiles {
    static var folder: URL {
        let u = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("receipts", isDirectory: true)
        try? FileManager.default.createDirectory(at: u, withIntermediateDirectories: true)
        return u
    }
    static func url(_ name: String) -> URL? { Vault.safeFileName(name).map { folder.appendingPathComponent($0) } }
    static func image(_ name: String) -> UIImage? { url(name).flatMap { UIImage(contentsOfFile: $0.path) } }
    static func remove(_ name: String) { if let u = url(name) { try? FileManager.default.removeItem(at: u) } }

    /// A JPEG no larger than 1600 points on its longest side, so a vault of photos stays small.
    static func save(_ data: Data) -> String? {
        guard let img = UIImage(data: data) else { return nil }
        let longest = max(img.size.width, img.size.height), scale = min(1, 1600 / max(longest, 1))
        let size = CGSize(width: img.size.width * scale, height: img.size.height * scale)
        let small = UIGraphicsImageRenderer(size: size).image { _ in img.draw(in: CGRect(origin: .zero, size: size)) }
        guard let jpeg = small.jpegData(compressionQuality: 0.7) else { return nil }
        let name = UUID().uuidString + ".jpg"
        do { try jpeg.write(to: folder.appendingPathComponent(name), options: .atomic); return name } catch { return nil }
    }
}

/// A note, tags and a photo for one payment, kept on this phone.
struct VaultCard: View {
    @EnvironmentObject var store: Store
    let id: String
    @State private var pick: PhotosPickerItem?
    @State private var problem: String?

    var body: some View {
        let e = store.vault.entry(id)
        GlassCard {
            VStack(alignment: .leading, spacing: 10) {
                Label("Your notes", systemImage: "note.text").vfont(15, .semibold).foregroundStyle(VUI.textPrimary)
                TextField("Note (optional)", text: Binding(get: { store.vault.entry(id).note }, set: { store.vault.setNote(id, $0) }), axis: .vertical)
                    .foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                TextField("Tags: trip, tax", text: Binding(get: { store.vault.entry(id).tags.joined(separator: ", ") }, set: { store.vault.setTags(id, $0) }))
                    .textInputAutocapitalization(.never).autocorrectionDisabled().foregroundStyle(VUI.textPrimary)
                    .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                if let name = e.photo, let img = ReceiptFiles.image(name) {
                    Image(uiImage: img).resizable().scaledToFit().frame(maxHeight: 220).clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous)).accessibilityLabel("Photo of the bill")
                    Button("Remove photo", role: .destructive) { ReceiptFiles.remove(name); store.vault.setPhoto(id, nil) }.vfont(14, .semibold)
                } else {
                    PhotosPicker(selection: $pick, matching: .images) { Label("Add a photo of the bill", systemImage: "camera") }.vfont(14, .semibold).foregroundStyle(VUI.violet)
                }
                if let problem { Text(problem).vfont(13).foregroundStyle(VUI.coral) }
                Text("Stays on this phone. Search finds tags and notes in History.").vfont(12).foregroundStyle(VUI.textTertiary)
            }
        }
        .onChange(of: pick) { _, item in
            guard let item else { return }
            Task {
                if let data = try? await item.loadTransferable(type: Data.self), let name = ReceiptFiles.save(data) {
                    if let old = store.vault.entry(id).photo { ReceiptFiles.remove(old) }
                    store.vault.setPhoto(id, name); problem = nil
                } else { problem = "Couldn't read that photo." }
                pick = nil
            }
        }
    }
}

/// A one-page PDF of a month's paid payments.
enum MonthPDF {
    static func make(_ r: MonthReport) -> URL? {
        let page = CGRect(x: 0, y: 0, width: 595, height: 842)
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("vayora-pay-\(r.title.replacingOccurrences(of: " ", with: "-")).pdf")
        let renderer = UIGraphicsPDFRenderer(bounds: page)
        let head = [NSAttributedString.Key.font: UIFont.boldSystemFont(ofSize: 20)], body = [NSAttributedString.Key.font: UIFont.systemFont(ofSize: 11)]
        let bold = [NSAttributedString.Key.font: UIFont.boldSystemFont(ofSize: 12)]
        do {
            try renderer.writePDF(to: url) { ctx in
                var y: CGFloat = 0
                func newPage() { ctx.beginPage(); y = 48; ("Vayora Pay (TEST) · \(r.title)" as NSString).draw(at: CGPoint(x: 40, y: y), withAttributes: head); y += 34 }
                newPage()
                ("Paid in the month: \(Money(minor: r.totalMinor).display)" as NSString).draw(at: CGPoint(x: 40, y: y), withAttributes: bold); y += 22
                for c in r.byCategory { ("\(c.category.title): \(Money(minor: c.minor).display)" as NSString).draw(at: CGPoint(x: 52, y: y), withAttributes: body); y += 16 }
                y += 10
                for l in r.lines {
                    if y > 800 { newPage() }
                    let line = "\(l.date.formatted(date: .abbreviated, time: .omitted))   \(l.name.isEmpty ? l.payee : l.name)   \(Money(minor: l.amountMinor).display)   \(l.category.title)"
                    (line as NSString).draw(at: CGPoint(x: 40, y: y), withAttributes: body); y += 15
                }
                y += 14; ("Test money. No real money moved. Not a bank statement." as NSString).draw(at: CGPoint(x: 40, y: min(y, 810)), withAttributes: body)
            }
            return url
        } catch { return nil }
    }
}
