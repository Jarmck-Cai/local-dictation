import SwiftUI
import UIKit

@main struct LocalDictationApp: App {
    var body: some Scene { WindowGroup { DictationView() } }
}

struct DictationView: View {
    @StateObject private var session = SessionCoordinator.shared
    @State private var consentDialog = false
    @State private var error: String?
    @State private var scratch = ""
    var body: some View {
        NavigationStack {
            Form {
                Section {
                    ModelStatusView(models: session.models)
                    Text(session.message).font(.callout)
                    HStack {
                        Button(session.isRecording ? "停止并转文字" : "开始听写", systemImage: session.isRecording ? "stop.fill" : "mic.fill") {
                            if session.isRecording { session.stop() }
                            else { Task { do { try await session.start() } catch { self.error = error.localizedDescription } } }
                        }
                        .disabled(!session.models.ready || session.state.job?.phase == .processing)
                        Spacer()
                        Button("取消", role: .cancel) { session.cancel() }
                    }
                } header: { Text("轻听写 · iPhone 原型") }
                Section("自动文字增强") {
                    Toggle("必要时联网轻整理", isOn: Binding(get: { session.state.cloudAllowed }, set: { enabled in
                        if enabled { consentDialog = true } else { session.setConsent(false) }
                    }))
                    Text(session.serviceConfigured ? "服务已内置。仅上传本次识别文字；录音留在手机。断网或超时使用离线结果。" : "当前构建未连接增强服务，可完整验证离线识别。服务由开发者内置，用户无需填写 API。")
                        .font(.footnote).foregroundStyle(.secondary)
                }
                if let job = session.state.job {
                    Section("本次文字") {
                        Text(job.text.isEmpty ? job.raw : job.text).textSelection(.enabled)
                        if !job.note.isEmpty { Text(job.note).font(.caption).foregroundStyle(.secondary) }
                        HStack {
                            Button("复制") { UIPasteboard.general.string = job.text.isEmpty ? job.raw : job.text }
                                .disabled(job.raw.isEmpty && job.text.isEmpty)
                            if job.phase == .failed { Button("重新识别本机录音") { session.recover() } }
                        }
                        if !job.raw.isEmpty && job.raw != job.text {
                            DisclosureGroup("查看离线原文") { Text(job.raw).textSelection(.enabled) }
                        }
                    }
                }
                Section("安装后验证") {
                    if session.appOnly {
                        Text("这是普通 Apple ID 可签名的应用内版本。先验证中英混说和按钮录音；跨应用落字需安装完整版本及 App Groups 签名。")
                    } else {
                        Text("1. 设置 → 通用 → 键盘 → 添加“轻听写”，并允许完全访问（用于本机共享文字）。")
                        Text("2. 动作按钮设置为快捷指令“开始或停止轻听写”；辅助触控也可配置运行这个快捷指令。")
                        Text("3. 在目标输入框切换到轻听写键盘，再按动作按钮开始。再次按下停止，手动返回原输入框，文字会插入一次。")
                        Text("输入框发生变化或无法确认位置时，改为点击“插入本次文字”。安全密码框、部分电话键盘或禁止第三方键盘的应用不支持。")
                    }
                    TextField("可在这里试用键盘落字", text: $scratch, axis: .vertical)
                    Text("模型约 242 MB，首次下载需要网络，之后识别可开飞行模式。录音最多 120 秒，超过一天的录音在打开应用时清理；可随时清除全部本机内容。")
                        .font(.footnote).foregroundStyle(.secondary)
                    Button("清除本机录音和本次文字", role: .destructive) { session.clear() }
                }
            }
            .navigationTitle("轻听写")
            .alert("允许自动联网增强？", isPresented: $consentDialog) {
                Button("允许，以后自动判断") { session.setConsent(true) }
                Button("保持离线", role: .cancel) {}
            } message: { Text("需要整理时会把识别文字发送到我们的 HTTPS 服务及其模型供应商，录音不会上传。允许一次后自动处理，可随时关闭。结果可能有误，请在发送前确认。") }
            .alert("暂时无法开始", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
                Button("知道了") { error = nil }
            } message: { Text(error ?? "") }
        }
    }
}

private struct ModelStatusView: View {
    @ObservedObject var models: ModelManager
    @State private var error: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label(models.ready ? "离线模型已就绪" : "准备离线模型", systemImage: models.ready ? "checkmark.circle" : "arrow.down.circle")
            Text(models.status).font(.caption).foregroundStyle(.secondary)
            if !models.ready {
                Button(models.busy ? "正在准备…" : "下载并校验离线模型") {
                    Task { do { try await models.prepare() } catch { self.error = error.localizedDescription } }
                }.disabled(models.busy)
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
        }
    }
}
