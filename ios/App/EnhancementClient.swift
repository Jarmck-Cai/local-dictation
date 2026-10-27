import Foundation
import LocalDictationCore

@MainActor final class EnhancementClient {
    private var task: Task<String?, Never>?
    private let endpoint: URL?
    private let token: String
    var configured: Bool { endpoint != nil && !token.isEmpty }
    init() {
        let url = Bundle.main.url(forResource: "Service", withExtension: "plist")
        let values = url.flatMap { NSDictionary(contentsOf: $0) } ?? [:]
        let candidate = URL(string: values["Endpoint"] as? String ?? "")
        endpoint = candidate?.scheme == "https" && candidate?.host != nil ? candidate : nil
        token = values["ClientToken"] as? String ?? ""
    }
    func cancel() { task?.cancel(); task = nil }
    func enhance(id: UUID, text: String) async -> String? {
        guard let endpoint, configured else { return nil }
        cancel()
        let token = token
        let requestTask = Task<String?, Never> {
            var request = URLRequest(url: endpoint)
            request.httpMethod = "POST"
            request.setValue("Bearer " + token, forHTTPHeaderField: "Authorization")
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try? JSONEncoder().encode(Payload(requestId: id.uuidString, text: text))
            request.timeoutInterval = 2
            let config = URLSessionConfiguration.ephemeral
            config.timeoutIntervalForRequest = 2; config.timeoutIntervalForResource = 2
            config.urlCache = nil; config.httpShouldSetCookies = false
            // Reject redirects: never forward the prototype token to another endpoint.
            let session = URLSession(configuration: config, delegate: NoRedirect(), delegateQueue: nil)
            defer { session.invalidateAndCancel() }
            let started = ContinuousClock.now
            do {
                let (data, response) = try await session.data(for: request)
                guard !Task.isCancelled, started.duration(to: .now) < .seconds(2),
                      (response as? HTTPURLResponse)?.statusCode == 200, data.count <= 65_536,
                      let reply = try? JSONDecoder().decode(Payload.self, from: data),
                      reply.requestId == id.uuidString,
                      EnhancementPolicy.accepts(original: text, candidate: reply.text) else { return nil }
                return reply.text
            } catch { return nil }
        }
        task = requestTask
        return await requestTask.value
    }
    private struct Payload: Codable { var requestId: String; var text: String }
    private final class NoRedirect: NSObject, URLSessionTaskDelegate {
        func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) { completionHandler(nil) }
    }
}
