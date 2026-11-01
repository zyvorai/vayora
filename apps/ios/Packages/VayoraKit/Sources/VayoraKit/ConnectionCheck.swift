import Foundation

/// What a "Test connection" press found, in words a person can act on.
public enum ConnectionVerdict: Equatable, Sendable {
    case ok
    case badAddress
    case thisPhone
    case cannotReach
    case timedOut
    case plainHTTPBlocked
    case notVayora(Int)
    case wrongKey
    case noKey
    case serverError(Int)

    public var isOK: Bool { self == .ok }

    public var title: String {
        switch self {
        case .ok: return "Connected"
        case .badAddress: return "That address doesn't look right"
        case .thisPhone: return "That address is this phone"
        case .cannotReach: return "Can't reach the server"
        case .timedOut: return "The server didn't answer"
        case .plainHTTPBlocked: return "iOS blocked a plain http address"
        case .notVayora: return "Something answered, but not a Vayora server"
        case .wrongKey: return "The server rejected the key"
        case .noKey: return "The server is reachable, but there is no key"
        case .serverError: return "The server had a problem"
        }
    }

    public var advice: String {
        switch self {
        case .ok: return "The server answered and the key was accepted. Test money only."
        case .badAddress: return "Use the form http://192.168.1.13:18080, with http:// or https:// and the port."
        case .thisPhone: return "127.0.0.1 and localhost mean this iPhone. Use the computer's address on your Wi-Fi, for example http://192.168.1.13:18080."
        case .cannotReach: return "Check the phone is on the same Wi-Fi as the computer, and that Settings, this app, Local Network is switched on. If Safari on this phone opens the address's /healthz, the Local Network switch is the likely cause."
        case .timedOut: return "The address is reachable in form but nothing replied. Check the server is running and the port is right."
        case .plainHTTPBlocked: return "iOS only allows plain http to addresses on your own network. Use the computer's local address, or an https address."
        case .notVayora(let s): return "The address replied with status \(s) but not like a Vayora server (/healthz). Check the port."
        case .wrongKey: return "Scan the setup QR again, or paste a test key that starts with zk_test_."
        case .noKey: return "Scan the setup QR, or paste a test key that starts with zk_test_ into the API key box."
        case .serverError(let s): return "The server replied with status \(s). Look at its log."
        }
    }
}

/// One probe's raw result: an HTTP status, or the URL error that stopped it.
public enum Probe: Equatable, Sendable {
    case status(Int)
    case failure(Int) // a URLError code raw value
}

public enum ConnectionCheck {
    /// Parses what a person typed. Returns nil when it cannot be a server address.
    public static func parse(_ text: String) -> URL? {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let u = URL(string: t), let scheme = u.scheme?.lowercased(), scheme == "http" || scheme == "https",
              let host = u.host, !host.isEmpty else { return nil }
        return u
    }

    public static func isThisPhone(_ url: URL) -> Bool {
        guard let h = url.host?.lowercased() else { return false }
        return h == "localhost" || h == "127.0.0.1" || h == "::1" || h == "[::1]"
    }

    /// `health` is GET /healthz. `auth` is a keyed call, or nil when the app has no key (Pay).
    public static func classify(address: URL?, health: Probe, auth: Probe?, needsKey: Bool = false) -> ConnectionVerdict {
        guard let address else { return .badAddress }
        if isThisPhone(address) { return .thisPhone }
        switch health {
        case .failure(let code):
            switch code {
            case URLError.timedOut.rawValue: return .timedOut
            case URLError.appTransportSecurityRequiresSecureConnection.rawValue: return .plainHTTPBlocked
            case URLError.badURL.rawValue, URLError.unsupportedURL.rawValue: return .badAddress
            default: return .cannotReach
            }
        case .status(let s):
            guard s == 200 else { return .notVayora(s) }
        }
        guard let auth else { return needsKey ? .noKey : .ok }
        switch auth {
        case .failure(let code): return code == URLError.timedOut.rawValue ? .timedOut : .cannotReach
        case .status(let s):
            if (200..<300).contains(s) { return .ok }
            if s == 401 || s == 403 { return .wrongKey }
            return s >= 500 ? .serverError(s) : .notVayora(s)
        }
    }
}

extension VayoraClient {
    private func probe(_ r: URLRequest) async -> Probe {
        do {
            let (_, resp) = try await session.data(for: r)
            return .status((resp as? HTTPURLResponse)?.statusCode ?? 0)
        } catch let e as URLError { return .failure(e.code.rawValue) }
        catch { return .failure(URLError.unknown.rawValue) }
    }

    /// Reaches the server, then (when a key is set) makes one small keyed call.
    public func checkConnection(needsKey: Bool = false) async -> ConnectionVerdict {
        var h = URLRequest(url: baseURL.appendingPathComponent("healthz")); h.timeoutInterval = 6
        let health = await probe(h)
        var auth: Probe?
        if case .status(200) = health, apiKey?.isEmpty == false {
            var a = request("GET", "/payment_intents?limit=1"); a.timeoutInterval = 6
            auth = await probe(a)
        }
        return ConnectionCheck.classify(address: baseURL, health: health, auth: auth, needsKey: needsKey)
    }
}
