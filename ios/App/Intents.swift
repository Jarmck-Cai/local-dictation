import AppIntents

struct ToggleDictationIntent: AudioRecordingIntent, ForegroundContinuableIntent {
    static let title: LocalizedStringResource = "开始或停止轻听写"
    static let description = IntentDescription("动作按钮切换录音；首次开始需要应用进入前台，停止后离线识别。")
    @MainActor func perform() async throws -> some IntentResult {
        let coordinator = SessionCoordinator.shared
        if coordinator.isRecording { coordinator.stop() }
        else {
            try await requestToContinueInForeground {
                try await coordinator.start()
            }
        }
        return .result()
    }
}

struct DictationShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: ToggleDictationIntent(), phrases: ["使用\(.applicationName)听写"], shortTitle: "开始或停止听写", systemImageName: "mic")
    }
}
