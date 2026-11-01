import AVFoundation
import Speech
import SwiftUI
import VayoraKit
import VayoraUI

/// Listens for a few seconds and turns what it heard into text. Prefers on-device recognition. Nothing is stored.
@MainActor
final class SpeechListener: ObservableObject {
    @Published var transcript = ""
    @Published var listening = false
    @Published var problem: String?
    private let engine = AVAudioEngine()
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?
    private var stopper: Task<Void, Never>?

    func start(locale: String) {
        problem = nil; transcript = ""
        SFSpeechRecognizer.requestAuthorization { status in
            DispatchQueue.main.async {
                guard status == .authorized else { self.problem = "Speech recognition is off for Vayora Pay. Turn it on in iPhone Settings."; return }
                AVAudioApplication.requestRecordPermission { ok in DispatchQueue.main.async { ok ? self.begin(locale) : (self.problem = "The microphone is off for Vayora Pay. Turn it on in iPhone Settings.") } }
            }
        }
    }

    private func begin(_ locale: String) {
        guard let rec = SFSpeechRecognizer(locale: Locale(identifier: locale)), rec.isAvailable else { problem = "Speech recognition isn't available for that language right now."; return }
        do {
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.record, mode: .measurement, options: .duckOthers); try session.setActive(true, options: .notifyOthersOnDeactivation)
            let req = SFSpeechAudioBufferRecognitionRequest()
            req.shouldReportPartialResults = true
            if rec.supportsOnDeviceRecognition { req.requiresOnDeviceRecognition = true }
            request = req
            let input = engine.inputNode
            input.removeTap(onBus: 0)
            input.installTap(onBus: 0, bufferSize: 1024, format: input.outputFormat(forBus: 0)) { buf, _ in req.append(buf) }
            engine.prepare(); try engine.start()
            listening = true
            task = rec.recognitionTask(with: req) { [weak self] result, error in
                DispatchQueue.main.async {
                    if let r = result { self?.transcript = r.bestTranscription.formattedString }
                    if error != nil || result?.isFinal == true { self?.stop() }
                }
            }
            stopper = Task { try? await Task.sleep(nanoseconds: 8_000_000_000); stop() }   // never listens longer than 8 seconds
        } catch { problem = "Couldn't start listening."; stop() }
    }

    func stop() {
        stopper?.cancel(); stopper = nil
        if engine.isRunning { engine.stop(); engine.inputNode.removeTap(onBus: 0) }
        request?.endAudio(); task?.cancel(); request = nil; task = nil
        listening = false
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }
}

/// "Pay Asha 200 rupees": speak it, see what was heard, and open the normal review. It never pays by itself.
struct VoiceView: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    @StateObject private var ear = SpeechListener()
    @State private var hindi = false
    @State private var paying: Pay?

    struct Pay: Identifiable { let payee: Payee; let amount: Money; var id: String { payee.vpa } }

    var result: VoiceCommand.Result? { ear.transcript.isEmpty || ear.listening ? nil : VoiceCommand.parse(ear.transcript, payees: store.payees.all) }

    var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.25)
            VStack(spacing: 18) {
                HStack { Text("Speak a payment").vfont(24, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader); Spacer()
                    Button { ear.stop(); dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close") }
                Picker("Language", selection: $hindi) { Text("English").tag(false); Text("हिन्दी").tag(true) }.pickerStyle(.segmented).disabled(ear.listening)
                Spacer()
                Button { ear.listening ? ear.stop() : ear.start(locale: hindi ? "hi-IN" : "en-IN") } label: {
                    Image(systemName: ear.listening ? "stop.fill" : "mic.fill").font(.system(size: 40, weight: .bold)).foregroundStyle(.white).frame(width: 110, height: 110)
                        .background(Brand.pay.gradient, in: Circle()).shadow(color: VUI.indigo.opacity(0.5), radius: 24, y: 8)
                }.buttonStyle(PressScale()).accessibilityLabel(ear.listening ? "Stop listening" : "Start listening")
                Text(ear.listening ? "Listening…" : "Say, for example, “Pay Asha 200 rupees”").vfont(15).foregroundStyle(VUI.textSecondary).multilineTextAlignment(.center)
                if !ear.transcript.isEmpty { Text("“\(ear.transcript)”").vfont(18, .semibold).foregroundStyle(VUI.textPrimary).multilineTextAlignment(.center) }
                if let p = ear.problem { Text(p).vfont(13).foregroundStyle(VUI.coral).multilineTextAlignment(.center) }
                if let r = result { outcome(r) }
                Spacer()
                Text("It only opens the review for someone you saved. You still slide and enter your PIN. Test money.").vfont(12).foregroundStyle(VUI.textTertiary).multilineTextAlignment(.center)
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
        .onDisappear { ear.stop() }
        .sheet(item: $paying) { p in
            PaySheet(payload: UPIPayload(payee: p.payee.vpa, name: p.payee.name, amount: p.amount, note: ""), transfer: true).environmentObject(store)
        }
    }

    @ViewBuilder func outcome(_ r: VoiceCommand.Result) -> some View {
        switch r {
        case .pay(let p, let a): action("Review \(a.display) to \(p.name)") { paying = Pay(payee: p, amount: a) }
        case .ambiguous(let ps, let a):
            Text("Which one?").vfont(14, .semibold).foregroundStyle(VUI.textSecondary)
            ForEach(ps) { p in action("\(p.name) · \(a.display)") { paying = Pay(payee: p, amount: a) } }
        case .noPayee(let n): Text(n.isEmpty ? "I didn't catch who to pay." : "No saved payee called “\(n)”. Add them in Payees first.").vfont(14).foregroundStyle(VUI.amber).multilineTextAlignment(.center)
        case .noAmount: Text("I didn't catch the amount.").vfont(14).foregroundStyle(VUI.amber)
        case .notAPayment: Text("That didn't sound like a payment. Try “Pay Asha 200 rupees”.").vfont(14).foregroundStyle(VUI.amber).multilineTextAlignment(.center)
        }
    }
    func action(_ t: String, _ run: @escaping () -> Void) -> some View {
        Button(action: run) { Text(t).vfont(16, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 14).background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 20, style: .continuous)) }.buttonStyle(PressScale())
    }
}
