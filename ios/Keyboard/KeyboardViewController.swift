import UIKit
import CryptoKit
import LocalDictationCore

final class KeyboardViewController: UIInputViewController {
    private let status = UILabel()
    private var timer: Timer?
    private var store: SharedStateStore?
    private var visible = false
    override func viewDidLoad() {
        super.viewDidLoad()
        let stack = UIStackView(); stack.axis = .vertical; stack.spacing = 14
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -16),
            stack.topAnchor.constraint(equalTo: view.topAnchor, constant: 16),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: view.bottomAnchor, constant: -12),
            view.heightAnchor.constraint(greaterThanOrEqualToConstant: 190)
        ])
        status.numberOfLines = 3; status.font = .preferredFont(forTextStyle: .callout)
        stack.addArrangedSubview(status)
        let controls = UIStackView(); controls.spacing = 20; controls.distribution = .fillEqually
        controls.addArrangedSubview(button("停止听写", #selector(stop)))
        controls.addArrangedSubview(button("取消", #selector(cancel)))
        stack.addArrangedSubview(controls)
        stack.addArrangedSubview(button("插入本次文字", #selector(insertManually)))
        let row = UIStackView(); row.distribution = .fillEqually
        let next = UIButton(type: .system); next.setTitle("🌐 切换键盘", for: .normal)
        next.addTarget(self, action: #selector(handleInputModeList(from:with:)), for: .allTouchEvents)
        row.addArrangedSubview(next)
        row.addArrangedSubview(button("空格", #selector(space)))
        row.addArrangedSubview(button("⌫", #selector(backspace)))
        stack.addArrangedSubview(row)
    }
    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated); visible = true
        do { store = try SharedEnvironment.store() } catch { status.text = error.localizedDescription }
        publishContext(); poll()
        timer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in self?.poll() }
    }
    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated); visible = false; timer?.invalidate(); timer = nil
    }
    override func textDidChange(_ textInput: UITextInput?) {
        super.textDidChange(textInput); if visible { publishContext() }
    }
    private var fingerprint: String? {
        // documentIdentifier + cursor surroundings: fail closed if iOS hides context.
        let before = textDocumentProxy.documentContextBeforeInput
        let after = textDocumentProxy.documentContextAfterInput
        guard before != nil || after != nil || !textDocumentProxy.hasText else { return nil }
        let value = textDocumentProxy.documentIdentifier.uuidString + "\u{0}" + (before ?? "") + "\u{0}" + (after ?? "")
        return SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined()
    }
    private func publishContext() {
        guard hasFullAccess else { return }
        do { try store?.update { $0.context = fingerprint.map { InputContext(fingerprint: $0) } } }
        catch { status.text = "共享文字失败，请检查完全访问和签名" }
    }
    private func poll() {
        guard visible else { return }
        guard hasFullAccess else { status.text = "请在系统键盘设置中允许完全访问，以共享本机识别文字。键盘不会读取剪贴板或发送录音。"; return }
        do {
            guard let state = try store?.read(), let job = state.job else {
                status.text = "按动作按钮开始听写，或先打开轻听写应用。"; return
            }
            switch job.phase {
            case .recording: status.text = "正在录音 · 按停止按钮结束"
            case .processing: status.text = "麦克风已关闭 · 正在处理"
            case .ready:
                status.text = "文字已准备好；位置变化时请点插入。"
                deliver(job.id, manual: false)
            case .claimed, .delivered: status.text = "本次文字已领取，可切回常用键盘。"
            case .cancelled: status.text = "已取消；按动作按钮开始下一次。"
            case .failed: status.text = job.note
            }
        } catch { status.text = "无法读取识别结果，请打开轻听写检查签名。" }
    }
    private func deliver(_ id: UUID, manual: Bool) {
        guard hasFullAccess, visible else { return }
        do {
            let context = fingerprint
            let text = try store?.update { $0.claim(id: id, context: context, manual: manual) }
            guard let text else { return }
            // iOS insertText has no success acknowledgement; never retry automatically.
            textDocumentProxy.insertText(text)
            try store?.update { if $0.job?.id == id { $0.job?.phase = .delivered } }
            publishContext()
        } catch { status.text = "落字中断，请回应用复制文字。" }
    }
    @objc private func insertManually() {
        guard let state = try? store?.read(), let id = state.job?.id else { return }
        deliver(id, manual: true)
    }
    @objc private func stop() { try? store?.update { if $0.job?.phase == .recording { $0.job?.command = "stop" } } }
    @objc private func cancel() { try? store?.update { if let id = $0.job?.id { $0.cancel(id: id) } } }
    @objc private func space() { textDocumentProxy.insertText(" "); publishContext() }
    @objc private func backspace() { textDocumentProxy.deleteBackward(); publishContext() }
    private func button(_ text: String, _ action: Selector) -> UIButton {
        let button = UIButton(type: .system); button.setTitle(text, for: .normal)
        button.addTarget(self, action: action, for: .touchUpInside); return button
    }
}
