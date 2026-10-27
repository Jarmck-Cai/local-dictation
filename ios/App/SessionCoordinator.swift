import SwiftUI
import AVFoundation
import ActivityKit
import LocalDictationCore

@MainActor final class SessionCoordinator: ObservableObject {
    static let shared = SessionCoordinator()
    @Published var state = SharedState()
    @Published var message = "准备离线模型后即可开始"
    let models = ModelManager()
    private let recorder = AudioRecorder()
    private let recognizer = OfflineRecognizer()
    private let enhancer = EnhancementClient()
    private var processing: Task<Void, Never>?
    private var timer: Timer?
    private var activity: Activity<RecordingAttributes>?
    private var backgroundID: UIBackgroundTaskIdentifier = .invalid
    private var store: SharedStateStore?
    var isRecording: Bool { recorder.isRecording }
    var serviceConfigured: Bool { enhancer.configured }
    var appOnly: Bool { Bundle.main.object(forInfoDictionaryKey: "LDAppOnly") as? Bool == true }
    private var recordings: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("Recordings", isDirectory: true)
    }
    private init() {
        do {
            store = try SharedEnvironment.store()
            try FileManager.default.createDirectory(at: recordings, withIntermediateDirectories: true)
            try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: recordings.path)
            var values = URLResourceValues(); values.isExcludedFromBackup = true
            var directory = recordings; try directory.setResourceValues(values)
            // Interrupted jobs never become ready automatically after a restart.
            try store?.update { s in
                if s.job?.phase == .recording || s.job?.phase == .processing {
                    s.job?.phase = .failed; s.job?.note = "上次任务中断，可重新识别本机录音。"
                }
            }
            reload()
            pruneRecordings()
        } catch { message = error.localizedDescription }
        recorder.onInterrupted = { [weak self] in self?.stop() }
        timer = Timer.scheduledTimer(withTimeInterval: 0.2, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.poll() }
        }
    }
    func reload() {
        do { if let store { state = try store.read() } }
        catch { message = "共享状态读取失败：" + error.localizedDescription }
    }
    func setConsent(_ allowed: Bool) {
        if !allowed { enhancer.cancel() }
        do { try store?.update { $0.setConsent(allowed) }; reload() }
        catch { message = error.localizedDescription }
    }
    func start() async throws {
        guard let store else { throw failure("共享存储未准备好") }
        guard models.ready else { throw failure("请先下载并校验离线模型") }
        guard !isRecording, processing == nil else { throw failure("请等待当前任务结束") }
        let granted = await AVAudioApplication.requestRecordPermission()
        guard granted else { throw failure("请在系统设置中允许麦克风") }
        let job: DictationJob = try store.update { s in
            let context = s.context.flatMap { Date().timeIntervalSince($0.updatedAt) < 60 ? $0.fingerprint : nil }
            let job = DictationJob(context: context, consentRevision: s.consentRevision)
            s.job = job; return job
        }
        do {
            if !appOnly {
                guard ActivityAuthorizationInfo().areActivitiesEnabled else { throw failure("请允许实时活动，以便随时停止录音") }
                activity = try Activity.request(attributes: RecordingAttributes(jobID: job.id.uuidString), content: ActivityContent(state: .init(startedAt: Date()), staleDate: Date().addingTimeInterval(125)), pushType: nil)
            }
            try recorder.start(at: recordings.appendingPathComponent(job.audioName))
            message = "正在录音，按动作按钮或停止按钮结束（最长 120 秒）"
        } catch {
            _ = recorder.stop(); await endActivity()
            try? store.update { $0.cancel(id: job.id) }
            reload(); throw error
        }
        reload()
    }
    func stop() {
        guard state.job?.phase == .recording, processing == nil, let id = state.job?.id else { return }
        let file = recorder.stop() // release mic before creating the ASR task
        Task { await endActivity() }
        guard let file else { fail(id, "录音未保存"); return }
        beginProcessing(id: id, file: file)
    }
    func cancel() {
        if let id = state.job?.id { try? store?.update { $0.cancel(id: id) } }
        _ = recorder.stop(); processing?.cancel(); enhancer.cancel()
        Task { await endActivity() }
        endBackground(); reload(); message = "已取消，未落字"
    }
    func recover() {
        guard processing == nil, !isRecording, models.ready, let old = state.job else { return }
        // Recovery needs a new explicit insertion; old context is intentionally discarded.
        try? store?.update { s in
            s.job?.ticket.context = nil; s.job?.ticket.claimed = false
            s.job?.ticket.cancelled = false
        }
        beginProcessing(id: old.id, file: recordings.appendingPathComponent(old.audioName))
    }
    func clear() {
        cancel()
        try? store?.update { $0.job = nil; $0.context = nil }
        for url in (try? FileManager.default.contentsOfDirectory(at: recordings, includingPropertiesForKeys: nil)) ?? [] { try? FileManager.default.removeItem(at: url) }
        reload(); message = "本机录音和文字已清除"
    }
    private func beginProcessing(id: UUID, file: URL) {
        do {
            try store?.update { s in
                guard s.job?.id == id else { return }
                s.job?.phase = .processing; s.job?.command = nil
            }
        } catch { fail(id, error.localizedDescription); return }
        reload(); message = "麦克风已关闭，正在离线识别"
        backgroundID = UIApplication.shared.beginBackgroundTask(withName: "OfflineDictation") { [weak self] in
            Task { @MainActor in
                self?.processing?.cancel(); self?.enhancer.cancel()
                self?.fail(id, "后台处理时间已用尽，请打开应用重新识别")
                self?.endBackground()
            }
        }
        processing = Task { [weak self] in
            guard let self else { return }
            defer { self.processing = nil; self.endBackground() }
            do {
                let raw = try await self.recognizer.transcribe(file: file, models: self.models.directory)
                try Task.checkCancellation()
                guard !raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw self.failure("没有检测到语音") }
                guard let snapshot = try self.store?.read(), snapshot.job?.id == id, snapshot.job?.phase == .processing else { return }
                // Save raw before any network; interruption never destroys local ASR.
                try self.store?.update { if $0.job?.id == id { $0.job?.raw = raw } }
                let revision = snapshot.consentRevision
                let useCloud = EnhancementPolicy.shouldEnhance(text: raw, allowed: snapshot.cloudAllowed, configured: self.enhancer.configured)
                let enhanced = useCloud ? await self.enhancer.enhance(id: id, text: raw) : nil
                try Task.checkCancellation()
                try self.store?.update { $0.finish(id: id, raw: raw, enhanced: enhanced, revision: revision, note: enhanced == nil ? "离线结果" : "文字已联网轻整理") }
                self.reload(); self.message = "文字已准备好，返回原输入框即可落字"
            } catch is CancellationError { /* cancelled job remains terminal */ }
            catch { self.fail(id, error.localizedDescription) }
        }
    }
    private func poll() {
        reload()
        guard let job = state.job else { return }
        if job.command == "cancel" && (isRecording || processing != nil) { cancel() }
        else if job.command == "stop", isRecording { stop() }
    }
    private func fail(_ id: UUID, _ note: String) {
        try? store?.update { s in
            guard s.job?.id == id, s.job?.phase != .cancelled else { return }
            s.job?.phase = .failed; s.job?.note = note
        }
        reload(); message = note
    }
    private func endActivity() async {
        let current = activity; activity = nil
        await current?.end(nil, dismissalPolicy: .immediate)
        // Clear orphan recording activities after process restart too.
        for other in Activity<RecordingAttributes>.activities { await other.end(nil, dismissalPolicy: .immediate) }
    }
    private func endBackground() {
        if backgroundID != .invalid { UIApplication.shared.endBackgroundTask(backgroundID); backgroundID = .invalid }
    }
    private func pruneRecordings() {
        for file in (try? FileManager.default.contentsOfDirectory(at: recordings, includingPropertiesForKeys: [.contentModificationDateKey])) ?? [] {
            let date = (try? file.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
            if Date().timeIntervalSince(date) > 86_400 { try? FileManager.default.removeItem(at: file) }
        }
    }
    private func failure(_ message: String) -> NSError { NSError(domain: "Dictation", code: 2, userInfo: [NSLocalizedDescriptionKey: message]) }
}
