import Foundation

public enum EnhancementPolicy {
    public static func shouldEnhance(text: String, allowed: Bool, configured: Bool) -> Bool {
        guard allowed, configured, text.count <= 8_000 else { return false }
        let compact = text.filter { !$0.isWhitespace }
        return compact.count >= 32 || (compact.count >= 16 && text.contains("嗯"))
    }
    // Conservative syntactic guard; it cannot prove semantic equivalence.
    public static func accepts(original: String, candidate: String) -> Bool {
        let trimmed = candidate.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed.count <= 8_000,
              trimmed.count <= original.count * 2 + 20,
              trimmed.count >= max(1, original.count / 2) else { return false }
        let pattern = #"[A-Za-z][A-Za-z0-9_./+#'-]*|[0-9]+(?:[.,:%/-][0-9]+)*|不要|不能|没有|不是|不会|不|没|未|无|别|非"#
        return protectedTokens(original, pattern) == protectedTokens(trimmed, pattern)
    }
    private static func protectedTokens(_ value: String, _ pattern: String) -> [String] {
        let regex = try! NSRegularExpression(pattern: pattern)
        let text = value as NSString
        return regex.matches(in: value, range: NSRange(location: 0, length: text.length))
            .map { text.substring(with: $0.range).lowercased() }.sorted()
    }
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
    public mutating func claim(id: UUID, context: String?, now: Date) -> Bool {
        guard self.id == id, !claimed, !cancelled,
              let expected = self.context, !expected.isEmpty, expected == context,
              (0...600).contains(now.timeIntervalSince(createdAt)) else { return false }
        claimed = true
        return true
    }
}
