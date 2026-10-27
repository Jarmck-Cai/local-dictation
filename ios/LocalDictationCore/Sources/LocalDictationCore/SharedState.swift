import Foundation
import Darwin

public enum JobPhase: String, Codable, Sendable {
    case recording, processing, ready, claimed, delivered, cancelled, failed
}
public struct InputContext: Codable, Sendable {
    public var fingerprint: String
    public var updatedAt: Date
    public init(fingerprint: String, updatedAt: Date = Date()) {
        self.fingerprint = fingerprint; self.updatedAt = updatedAt
    }
}
public struct DictationJob: Codable, Sendable {
    public var ticket: DeliveryTicket
    public var phase: JobPhase
    public var audioName: String
    public var raw: String = ""
    public var text: String = ""
    public var note: String = ""
    public var command: String?
    public var consentRevision: Int
    public var id: UUID { ticket.id }
    public init(context: String?, consentRevision: Int) {
        ticket = DeliveryTicket(createdAt: Date(), context: context)
        phase = .recording; audioName = ticket.id.uuidString + ".wav"
        self.consentRevision = consentRevision
    }
}
public struct SharedState: Codable, Sendable {
    public var cloudAllowed = false
    public var consentRevision = 0
    public var context: InputContext?
    public var job: DictationJob?
    public init() {}
    public mutating func setConsent(_ allowed: Bool) {
        cloudAllowed = allowed; consentRevision += 1
        // A revoked enhanced result which has not been delivered reverts to raw.
        if !allowed, job?.phase == .ready {
            let raw = job?.raw ?? ""
            job?.text = raw
        }
    }
    public mutating func cancel(id: UUID) {
        guard job?.id == id else { return }
        job?.ticket.cancelled = true; job?.phase = .cancelled; job?.command = "cancel"
    }
    public mutating func finish(id: UUID, raw: String, enhanced: String?, revision: Int, note: String) {
        guard job?.id == id, job?.phase == .processing, job?.ticket.cancelled == false else { return }
        let canUseCloud = cloudAllowed && consentRevision == revision
        job?.raw = raw; job?.text = canUseCloud ? (enhanced ?? raw) : raw
        job?.note = note; job?.phase = .ready
    }
    public mutating func claim(id: UUID, context: String?, now: Date = Date(), manual: Bool = false) -> String? {
        guard var current = job, current.id == id, current.phase == .ready,
              !current.text.isEmpty else { return nil }
        if manual {
            guard !current.ticket.cancelled, !current.ticket.claimed else { return nil }
            current.ticket.claimed = true
        } else if !current.ticket.claim(id: id, context: context, now: now) { return nil }
        current.phase = .claimed; job = current
        return current.text
    }
}

/// One locked read-modify-atomic-write across the app and both extensions.
/// Claim is persisted before insertText: a crash can lose an insertion, never retry it automatically.
public final class SharedStateStore: @unchecked Sendable {
    public let directory: URL
    private static let processLock = NSLock()
    public init(directory: URL) throws {
        self.directory = directory
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }
    public func read() throws -> SharedState { try access { $0 } }
    @discardableResult public func update<T>(_ change: (inout SharedState) throws -> T) throws -> T {
        try access(write: true, change)
    }
    private func access<T>(write: Bool = false, _ change: (inout SharedState) throws -> T) throws -> T {
        Self.processLock.lock(); defer { Self.processLock.unlock() }
        let descriptor = open(directory.appendingPathComponent("state.lock").path, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR)
        guard descriptor >= 0 else { throw CocoaError(.fileWriteNoPermission) }
        defer { close(descriptor) }
        guard flock(descriptor, LOCK_EX) == 0 else { throw CocoaError(.fileLocking) }
        defer { flock(descriptor, LOCK_UN) }
        let url = directory.appendingPathComponent("state.json")
        var state: SharedState
        if FileManager.default.fileExists(atPath: url.path) {
            // Corruption is surfaced; silently resetting would risk re-delivery.
            state = try JSONDecoder().decode(SharedState.self, from: Data(contentsOf: url))
        } else { state = SharedState() }
        let result = try change(&state)
        if write { try JSONEncoder().encode(state).write(to: url, options: .atomic) }
        return result
    }
}
