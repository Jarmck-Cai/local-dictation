import WidgetKit
import SwiftUI
import ActivityKit

@main struct DictationWidgets: WidgetBundle {
    var body: some Widget { DictationActivity() }
}

struct DictationActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: RecordingAttributes.self) { context in
            HStack {
                Image(systemName: "mic.fill").foregroundStyle(.red)
                Text("轻听写正在录音")
                Text(context.state.startedAt, style: .timer).monospacedDigit()
                Spacer()
                Button(intent: StopDictationIntent()) { Image(systemName: "stop.circle.fill").font(.title) }
            }.padding()
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) { Label("轻听写", systemImage: "mic.fill") }
                DynamicIslandExpandedRegion(.trailing) { Text(context.state.startedAt, style: .timer) }
                DynamicIslandExpandedRegion(.bottom) { Button("停止并转文字", intent: StopDictationIntent()) }
            } compactLeading: { Image(systemName: "mic.fill").foregroundStyle(.red) }
            compactTrailing: { Text("录音") }
            minimal: { Image(systemName: "mic.fill").foregroundStyle(.red) }
        }
    }
}
