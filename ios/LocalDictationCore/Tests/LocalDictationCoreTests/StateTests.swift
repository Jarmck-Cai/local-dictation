import XCTest
@testable import LocalDictationCore

final class StateTests: XCTestCase {
    func jobState() -> SharedState {
        var state = SharedState(); state.setConsent(true)
        state.job = DictationJob(context: "document", consentRevision: state.consentRevision)
        state.job?.phase = .processing
        return state
    }
    func testRevokedCloudCannotDeliverLateOrAlreadyReadyEnhancement() {
        var state = jobState(); let id = state.job!.id; let revision = state.consentRevision
        state.setConsent(false)
        state.finish(id: id, raw: "原文", enhanced: "润色", revision: revision, note: "")
        XCTAssertEqual(state.job?.text, "原文")
        state = jobState()
        state.finish(id: state.job!.id, raw: "原文", enhanced: "润色", revision: state.consentRevision, note: "")
        XCTAssertEqual(state.job?.text, "润色")
        state.setConsent(false)
        XCTAssertEqual(state.job?.text, "原文")
    }
    func testCancelNewJobAndReenabledConsentRejectOldResponse() {
        var state = jobState(); let old = state.job!.id; let revision = state.consentRevision
        state.cancel(id: old)
        state.finish(id: old, raw: "原文", enhanced: "润色", revision: revision, note: "")
        XCTAssertEqual(state.job?.phase, .cancelled)
        state = jobState(); let id = state.job!.id; let before = state.consentRevision
        state.setConsent(false); state.setConsent(true)
        state.finish(id: id, raw: "原文", enhanced: "润色", revision: before, note: "")
        XCTAssertEqual(state.job?.text, "原文")
        state.job = DictationJob(context: "new", consentRevision: state.consentRevision)
        state.finish(id: id, raw: "旧", enhanced: "旧", revision: before, note: "")
        XCTAssertEqual(state.job?.phase, .recording)
        XCTAssertEqual(state.job?.text, "")
    }
    func testPersistentStoreClaimIsAtomicAcrossInstances() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let first = try SharedStateStore(directory: root)
        let second = try SharedStateStore(directory: root)
        var state = jobState(); let id = state.job!.id
        state.finish(id: id, raw: "测试", enhanced: nil, revision: state.consentRevision, note: "")
        try first.update { $0 = state }
        XCTAssertEqual(try first.update { $0.claim(id: id, context: "document") }, "测试")
        XCTAssertNil(try second.update { $0.claim(id: id, context: "document") })
        XCTAssertEqual(try second.read().job?.phase, .claimed)
    }
}
