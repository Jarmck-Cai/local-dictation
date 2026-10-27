import Foundation

public enum EnhancementPolicy {
    public static func shouldEnhance(text: String, allowed: Bool, configured: Bool) -> Bool { false }
    public static func accepts(original: String, candidate: String) -> Bool { false }
}

public struct DeliveryTicket: Codable, Equatable, Sendable {
    public var id: UUID
    public var createdAt: Date
    public var context: String?
    public var claimed: Bool = false
    public var cancelled: Bool = false
    public init(id: UUID = UUID(), createdAt: Date, context: String?) {
        self.id = id; self.createdAt = createdAt; self.context = context
    }
    public mutating func claim(id: UUID, context: String?, now: Date) -> Bool { false }
}
