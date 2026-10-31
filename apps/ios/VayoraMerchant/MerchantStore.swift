import Foundation
import SwiftUI
import VayoraKit

@MainActor
final class MerchantStore: ObservableObject {
    @Published var serverURL: String { didSet { UserDefaults.standard.set(serverURL, forKey: "serverURL") } }
    @Published var apiKey: String { didSet { Keychain.set(apiKey, for: "apiKey") } }

    init() {
        serverURL = UserDefaults.standard.string(forKey: "serverURL") ?? "http://127.0.0.1:8080"
        apiKey = Keychain.get("apiKey") ?? ""
    }

    var client: VayoraClient? {
        guard !apiKey.isEmpty, let u = URL(string: serverURL) else { return nil }
        return VayoraClient(baseURL: u, apiKey: apiKey)
    }
}
