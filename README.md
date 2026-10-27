# Local Dictation

Windows 通用语音输入工具。本地 SenseVoice INT8 识别中英混合语音，可用 Claude 整理文字，也可切换本机整理服务或关闭整理。

按住 **F8** 说话，松键后将结果一次粘贴到当前光标位置，由使用者自行发送。适用于编辑器、终端、聊天等普通输入场景，无需按应用安装插件。

当前版本 **0.1.3**，属于 Windows 内测版。基于 [Handy v0.9.8](https://github.com/cjpais/Handy/releases/tag/v0.9.8) 改造，桌面端使用 Tauri 2、Rust 和 React，运行不依赖 Python。来源与许可见 [UPSTREAM.md](desktop/UPSTREAM.md) 和 [LICENSE](LICENSE)。

## 功能

- 本地识别，录音不上传；Claude 模式只发送转写文字。
- 默认轻整理、断句和自然分段，可调整强度，始终以保留事实和原意为目标。
- 云端短文等待预算三秒，随长度增加至最多八秒；超时回退本地，连续失败后自动暂停，可手动恢复。
- 个人词库、本地术语候选及主动提交修改稿，候选经确认后才生效。
- 显示识别、整理、粘贴及总等待时间，保留本地历史供复制取用。
- 录音时临时静音播放并恢复；录音本地缓存七天，安装版配置当前用户每日清理任务。

默认快捷键：F8 按住录音，Esc 取消，Ctrl+Shift+F8 复制最近结果，Ctrl+Alt+F8 暂停或恢复云端整理，均可配置。输入位置以文字实际粘贴时的当前光标为准，已有选区会被替换。

## 使用与构建

使用流程、凭据配置和数据策略见 [使用说明](docs/desktop-usage.md)。源码在 `desktop/`，Windows 构建需要 Rust stable、Visual Studio 2022 C++ Build Tools、Windows SDK 和 CMake。

在项目根目录的 PowerShell 中运行：

```powershell
./scripts/prepare-desktop.ps1
./scripts/build-desktop.ps1
./scripts/package-desktop.ps1
```

准备脚本下载并校验构建资源与模型，来源及摘要见 [固定资源](desktop/RESOURCES.md)。模型、依赖缓存和生成的安装包不存入 Git。构建产物位于 `outputs/desktop/`，可用 `run-desktop.cmd` 启动本机已安装版本或该目录的免安装版本。

桌面端 Claude 密钥在界面本机配置，使用 Windows 当前账户保护；本机整理需自行启动兼容服务。初次克隆不含任何密钥或用户数据。可选的 Python 测量工具见 [验证说明](docs/validation.md)，与桌面运行时分开。

## 开发状态

0.1.3 已完成构建与离线检查。实际识别质量、松键到屏幕落字时间、资源占用及不同应用的输入兼容性仍需日常反馈，旧 Python 测量不能代表桌面端表现。词库提取及整理护栏使用规则，无法保证发现所有术语或避免每次语义改写。

[版本记录](docs/releases.md) · [开发与推送流程](docs/development.md)
