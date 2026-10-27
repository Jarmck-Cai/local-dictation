import AppIntents
import LocalDictationCore

struct StopDictationIntent: AudioRecordingIntent {
    static let title: LocalizedStringResource = "停止轻听写"
    func perform() async throws -> some IntentResult {
        let store = try SharedEnvironment.store()
        try store.update { state in
            if state.job?.phase == .recording { state.job?.command = "stop" }
        }
        return .result()
    }
}
