import Foundation
import WebKit

/// Serves `ledgeline://localhost/...` by handing each request to the
/// in-process Ledgeline router (`LedgelineApp.handle`): no socket, no port.
///
/// The SPA must load from `ledgeline://localhost/` (Rust's `MOBILE_ORIGIN`):
/// `tokenInitScript()` hands the bearer token to that origin only.
struct LedgelineSchemeHandler: URLSchemeHandler {
    static let scheme = "ledgeline"
    static let host = "localhost"
    static let origin = URL(string: "ledgeline://localhost/")!

    let app: LedgelineApp

    func reply(for request: URLRequest) -> LedgelineReply {
        LedgelineReply(request: request, app: self.app)
    }
}

/// One response, pulled a chunk at a time: WebKit asks for the next piece only
/// when it wants it, and each piece is one `LedgelineResponse.nextChunk()`.
/// Iterated off the main actor; the Rust calls run on the journal's own
/// runtime.
struct LedgelineReply: AsyncSequence {
    typealias Element = URLSchemeTaskResult
    typealias Failure = any Error

    let request: URLRequest
    let app: LedgelineApp

    func makeAsyncIterator() -> LedgelineReplyIterator {
        LedgelineReplyIterator(request: self.request, app: self.app)
    }
}

/// `LedgelineReply`'s iterator: the response head first, then one chunk per
/// call, until `nextChunk()` returns `nil`. A chunk that errors fails the task.
struct LedgelineReplyIterator: AsyncIteratorProtocol {
    typealias Element = URLSchemeTaskResult
    typealias Failure = any Error

    let request: URLRequest
    let app: LedgelineApp
    private var body: LedgelineResponse?
    private var done = false
    private var bytes = 0
    private var summary = ""
    private let started = ContinuousClock.now

    init(request: URLRequest, app: LedgelineApp) {
        self.request = request
        self.app = app
    }

    mutating func next() async throws -> URLSchemeTaskResult? {
        if self.done { return nil }
        guard let body else {
            do {
                let (response, head) = try await start()
                self.body = response
                return .response(head)
            } catch {
                self.done = true
                Log.info("scheme: \(self.request.httpMethod ?? "GET") \(self.request.url?.path() ?? "?") failed: \(error)")
                throw error
            }
        }
        do {
            if let chunk = try await body.nextChunk() {
                self.bytes += chunk.count
                return .data(chunk)
            }
        } catch {
            self.done = true
            Log.info("\(self.summary) body broke off after \(self.bytes)B: \(error)")
            throw error
        }
        self.done = true
        #if DEBUG
            Log.info("\(self.summary) \(self.bytes)B in \(millis(since: self.started))ms")
        #endif
        return nil
    }

    private mutating func start() async throws -> (LedgelineResponse, URLResponse) {
        guard let url = request.url else { throw URLError(.badURL) }
        let method = self.request.httpMethod ?? "GET"
        let headers = (request.allHTTPHeaderFields ?? [:]).map { Header(name: $0.key, value: $0.value) }
        let response = try await app.handle(
            method: method,
            url: url.absoluteString,
            headers: headers,
            body: Self.body(of: self.request)
        )
        let status = Int(response.status())
        let fields = Self.fields(response.headers())
        // The path only: query strings can carry search terms and filters.
        self.summary = "\(method) \(url.path(percentEncoded: true)) -> \(status)"
        if status >= 500 {
            Log.info("\(self.summary) Retry-After=\(fields["Retry-After"] ?? "-")")
        }
        guard let head = HTTPURLResponse(url: url, statusCode: status, httpVersion: "HTTP/1.1", headerFields: fields)
        else { throw URLError(.cannotParseResponse) }
        return (response, head)
    }

    /// The request body, wherever WebKit put it: JSON string bodies arrive in
    /// `httpBody`, others (sometimes) as a stream.
    private static func body(of request: URLRequest) -> Data {
        if let data = request.httpBody { return data }
        guard let stream = request.httpBodyStream else { return Data() }
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 64 * 1024)
        while stream.hasBytesAvailable {
            let count = stream.read(&buffer, maxLength: buffer.count)
            if count <= 0 { break }
            data.append(buffer, count: count)
        }
        return data
    }

    /// Header list to a dictionary, keyed by canonical names so lookups work.
    /// Repeated headers fold with ", ".
    private static func fields(_ headers: [Header]) -> [String: String] {
        Dictionary(headers.map { (self.canonical($0.name), $0.value) }, uniquingKeysWith: { "\($0), \($1)" })
    }

    private static func canonical(_ name: String) -> String {
        name.split(separator: "-")
            .map { $0.prefix(1).uppercased() + $0.dropFirst().lowercased() }
            .joined(separator: "-")
    }
}
