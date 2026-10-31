import Foundation

/// What a "setup QR" carries: the server address and a **test** API key, so nobody has to type them on a phone.
/// Format: `vayora://setup?server=<url>&key=<zk_test_…>`. Live keys are refused: these apps are test-mode only.
public struct SetupPayload: Equatable, Sendable {
    public let server: URL
    public let apiKey: String

    public enum Failure: Error, Equatable { case notSetup, badServer, missingKey, liveKeyRefused }

    public static func parse(_ text: String) throws -> SetupPayload {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let c = URLComponents(string: t), c.scheme?.lowercased() == "vayora", c.host?.lowercased() == "setup" else { throw Failure.notSetup }
        let items = c.queryItems ?? []
        func one(_ n: String) -> String? {
            let v = items.filter { $0.name == n }
            return v.count == 1 ? v[0].value : nil
        }
        guard let s = one("server"), let url = URL(string: s), let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https", let host = url.host, !host.isEmpty else { throw Failure.badServer }
        guard let key = one("key"), !key.isEmpty else { throw Failure.missingKey }
        guard key.hasPrefix("zk_test_") else { throw Failure.liveKeyRefused }
        return SetupPayload(server: url, apiKey: key)
    }
}
