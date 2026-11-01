import AVFoundation
import VayoraKit

/// Speaks "250 rupees received" when a payment lands. Off by default; the phone's own voices, no network.
@MainActor
final class Soundbox {
    static let shared = Soundbox()
    private let synth = AVSpeechSynthesizer()

    static var language: SoundboxPhrase.Language? {
        switch UserDefaults.standard.string(forKey: "soundbox") ?? "off" {
        case "en": .english
        case "hi": .hindi
        default: nil
        }
    }

    func announce(_ amount: Money) {
        guard let lang = Self.language else { return }
        speak(SoundboxPhrase.received(amount, language: lang), lang)
    }

    func speak(_ text: String, _ lang: SoundboxPhrase.Language) {
        // play over the silent switch, like a counter speaker; stop any phrase still being spoken
        try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .spokenAudio, options: [.duckOthers])
        try? AVAudioSession.sharedInstance().setActive(true)
        synth.stopSpeaking(at: .immediate)
        let u = AVSpeechUtterance(string: text)
        u.voice = AVSpeechSynthesisVoice(language: lang.rawValue)
        u.rate = AVSpeechUtteranceDefaultSpeechRate * 0.9
        synth.speak(u)
    }
}
