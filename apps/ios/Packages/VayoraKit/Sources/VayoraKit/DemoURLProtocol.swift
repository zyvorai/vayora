import Foundation

/// Serves `DemoBank` to a `URLSession`, so `VayoraClient` runs unchanged against the built-in demo.
public final class DemoURLProtocol: URLProtocol {
    /// Set by `VayoraClient.demo(bank:)`; the one bank this process's demo requests are served from.
    nonisolated(unsafe) static var testBank: DemoBank?
    static var bank: DemoBank { testBank ?? .shared }

    public override class func canInit(with request: URLRequest) -> Bool { request.url?.host == DemoBank.host }
    public override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    public override func startLoading() {
        guard let url = request.url else { client?.urlProtocol(self, didFailWithError: URLError(.badURL)); return }
        var body = request.httpBody
        if body == nil, let stream = request.httpBodyStream {
            stream.open(); defer { stream.close() }
            var d = Data(); var buf = [UInt8](repeating: 0, count: 4096)
            while stream.hasBytesAvailable { let n = stream.read(&buf, maxLength: buf.count); if n <= 0 { break }; d.append(buf, count: n) }
            body = d
        }
        let r = Self.bank.handle(method: request.httpMethod ?? "GET", path: url.path, body: body)
        let resp = HTTPURLResponse(url: url, statusCode: r.status, httpVersion: "HTTP/1.1", headerFields: ["Content-Type": "application/json"])!
        client?.urlProtocol(self, didReceive: resp, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: r.data)
        client?.urlProtocolDidFinishLoading(self)
    }
    public override func stopLoading() {}
}

extension VayoraClient {
    /// A client wired to the built-in demo bank. No network is used.
    public static func demo(bank: DemoBank = .shared) -> VayoraClient {
        DemoURLProtocol.testBank = bank
        let c = URLSessionConfiguration.ephemeral
        c.protocolClasses = [DemoURLProtocol.self]
        return VayoraClient(baseURL: URL(string: "http://\(DemoBank.host)")!, apiKey: "zk_test_demo", session: URLSession(configuration: c))
    }
}
