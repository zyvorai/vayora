import Foundation
import SwiftUI
import VayoraKit

@MainActor
final class MerchantStore: ObservableObject {
    @Published var serverURL: String { didSet { UserDefaults.standard.set(serverURL, forKey: "serverURL"); Keychain.set(serverURL, for: "serverURL") } }
    @Published var apiKey: String { didSet { Keychain.set(apiKey, for: "apiKey") } }

    init() {
        let saved = Keychain.get("serverURL") ?? UserDefaults.standard.string(forKey: "serverURL") ?? "http://127.0.0.1:8080"
        serverURL = saved
        // an address saved by an older build lives only in UserDefaults: copy it to the Keychain so a reinstall keeps it
        if URL(string: saved)?.host != "127.0.0.1", Keychain.get("serverURL") == nil { Keychain.set(saved, for: "serverURL") }
        apiKey = Keychain.get("apiKey") ?? ""
    }

    var client: VayoraClient? {
        guard !apiKey.isEmpty, let u = URL(string: serverURL) else { return nil }
        return VayoraClient(baseURL: u, apiKey: apiKey)
    }
}
