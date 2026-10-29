# 远程窗口听写输入：隔离调查计划

## 范围与状态

本分支从 Beta 1 提交 `97d8c08bcb02d6763b324f4e90c5a64a911d7793` 分出，独立于会议功能后续修改。第一阶段只完成源码调查与验证计划，不修改生产输入算法。

目标场景：本机 Windows 运行听写，焦点位于远程控制窗口，希望正文进入远端当前编辑位置。已有反馈是其他听写产品能够完成同类操作；尚未对本应用完成远控环境复现，因此不能确定根因或承诺通用兼容。

开始下一阶段前，最小必需信息是远控软件名称与远端操作系统。版本、窗口/全屏模式、键盘映射和剪贴板重定向设置随后按具体产品确认。

## 已核对的源码事实

- `desktop/src-tauri/src/clipboard.rs` 的 `paste_automatic_windows` 调用 `protected_paste::run_with_preflight`，复用受保护的剪贴板事务和键盘预检。
- `desktop/src-tauri/src/manual_paste.rs` 的 `send_paste_chord` 支持 Ctrl+V、Ctrl+Shift+V、Shift+Insert，通过 Enigo 注入；当前没有远端系统专用快捷键选择。Windows 本机按键释放观察与注入前快速复查均为现有保护。
- `desktop/src-tauri/src/protected_paste_windows.rs` 发布延迟渲染的 Unicode 文本；注入开始后收到 `WM_RENDERFORMAT` 才记录消费者读取。发布完成后重新取得剪贴板序列号，注入前同时验证序列号和所有权。
- `desktop/src-tauri/src/protected_paste_policy.rs` 的观察上限为 3 秒，读取后安静 150 毫秒即可进入结算。结算根据许可、取消状态和复制偏好恢复快照、保留结果或保留新复制。
- `desktop/src-tauri/src/protected_paste.rs` 明确说明：观察到读取不能证明目标已经插入文字。现有计时结果也保留此区别。
- `clipboard.rs` 的 `validate_paste_method` 明确拒绝 Direct 和 ExternalScript，不能把现有内部入口视为已支持的远程输入方案。

## 待验证的候选解释

这些是源码支持的调查方向，不是实测结论：

1. 远控客户端可能在本机读取剪贴板并异步同步到远端。本机读取回执无法表明远端已接收、应用已读取或插入；恢复时序和同步时序可能存在交互。
2. 粘贴组合键可能被本机、远控客户端或远端拦截。若本机 Windows 连接远端 macOS，还需核对 Ctrl 到 Command 的映射策略。
3. 剪贴板重定向可能未启用、仅单向启用或不接受延迟渲染发布，需要依据具体软件的官方协议/设置说明核实。
4. 按键状态预检、焦点变化或远控特殊键处理可能在注入前拒绝输入，需先区分错误反馈和无提示未落字。

## 下一阶段路线

取得软件与远端系统信息后，先查对应官方说明，再选择最小复现路径。使用固定合成文本，分别区分剪贴板传播、远端手动粘贴、自动组合键转发与正文最终落字；每项结果记录环境、观察范围和未证实部分。实际远控连接、系统剪贴板读写、发键和听写录音不属于本阶段已执行事项。

实现优先复用 `protected_paste_policy::Provider` 事务接口、`manual_paste_keyboard::Keyboard` 注入接口及现有输出协调许可。确有需要时在这些边界增加明确的远控策略，不通过取消所有权校验、覆盖并发复制、增加盲目重试或无限等待来获得表面成功。仅在证据明确后决定是否需要应用配置、快捷键映射或新的投递适配器。

## 验证与验收

本次文档阶段只核对引用路径、源码事实、分支基线和公开内容；未运行应用、录音、读取或写入系统剪贴板、发出按键、连接远端或调用模型。

后续代码改变应先用合成适配器覆盖远控提前读取、异步传播、取消、并发复制和单次注入；复用 `scripts/check-automatic-paste.py`、`scripts/check-protected-paste.py`、`scripts/check-keyboard-state.py`、`scripts/check-win32-publication.py`。这些离线检查通过只能证明相应状态与保护逻辑，最终仍需在指定远控产品、远端系统和编辑应用完成真实落字验收。
