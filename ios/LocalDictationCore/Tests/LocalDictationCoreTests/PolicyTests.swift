import XCTest
@testable import LocalDictationCore

final class PolicyTests: XCTestCase {
    let mixed = "这个 project 的 deadline 是下周五，我们需要先检查 API 然后再安排一次 review"
    func testCloudRequiresConsentAndConfiguration() {
        XCTAssertFalse(EnhancementPolicy.shouldEnhance(text: mixed, allowed: false, configured: true))
        XCTAssertFalse(EnhancementPolicy.shouldEnhance(text: mixed, allowed: true, configured: false))
        XCTAssertTrue(EnhancementPolicy.shouldEnhance(text: mixed, allowed: true, configured: true))
        XCTAssertFalse(EnhancementPolicy.shouldEnhance(text: "你好", allowed: true, configured: true))
    }
    func testCloudCannotChangeNumbersNegationOrTechnicalWords() {
        XCTAssertTrue(EnhancementPolicy.accepts(original: "不要修改 API 的 16 个参数", candidate: "不要修改 API 的 16 个参数。"))
        XCTAssertFalse(EnhancementPolicy.accepts(original: "不要修改 API 的 16 个参数", candidate: "修改 API 的 16 个参数。"))
        XCTAssertFalse(EnhancementPolicy.accepts(original: "不要修改 API 的 16 个参数", candidate: "不要修改 API 的 17 个参数。"))
        XCTAssertFalse(EnhancementPolicy.accepts(original: "use Whisper API", candidate: "use Wispr API"))
        XCTAssertFalse(EnhancementPolicy.accepts(original: mixed, candidate: ""))
        XCTAssertFalse(EnhancementPolicy.accepts(original: mixed, candidate: String(repeating: mixed, count: 4)))
    }
    func testOnlyMatchingFreshContextCanClaimOnce() {
        let now = Date()
        var ticket = DeliveryTicket(createdAt: now, context: "before|after")
        XCTAssertFalse(ticket.claim(id: UUID(), context: "before|after", now: now))
        XCTAssertFalse(ticket.claim(id: ticket.id, context: "other", now: now))
        XCTAssertTrue(ticket.claim(id: ticket.id, context: "before|after", now: now))
        XCTAssertFalse(ticket.claim(id: ticket.id, context: "before|after", now: now))
    }
    func testMissingContextExpiredOrCancelledNeverAutoInserts() {
        let now = Date()
        var missing = DeliveryTicket(createdAt: now, context: nil)
        XCTAssertFalse(missing.claim(id: missing.id, context: nil, now: now))
        var old = DeliveryTicket(createdAt: now.addingTimeInterval(-601), context: "x")
        XCTAssertFalse(old.claim(id: old.id, context: "x", now: now))
        var cancelled = DeliveryTicket(createdAt: now, context: "x")
        cancelled.cancelled = true
        XCTAssertFalse(cancelled.claim(id: cancelled.id, context: "x", now: now))
    }
}
