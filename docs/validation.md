# 独立 Python 验证工具

本工具用于开发时测量 SenseVoice INT8 本地识别与 Claude API 文字整理，记录混说错误、停止录音到最终预览时间、CPU 和内存。它使用独立 Python 环境，不属于当前 Tauri/Rust 桌面版运行时，也不执行桌面版的自动输入。桌面版使用说明见 [desktop-usage.md](desktop-usage.md)。

## 直接试录

先按下文准备独立环境和模型。需要 Claude 整理时，将项目根目录的 `.env.example` 复制为 `.env.local`，仅在本机填入自己的 `ANTHROPIC_API_KEY`；不在聊天、截图或 Git 中保存密钥。默认模型为 `claude-haiku-5-5`，原厂地址为 `https://api.anthropic.com`。此配置与桌面版界面保存的凭据分别管理。

验证器拒绝 HTTP 重定向，本地整理只连接回环地址并禁用代理；数值保护包含数字正负号、小数和百分号，但仍不保证所有中文数字、单位和否定语义不被改动。

双击项目根目录 `run-validation.cmd`。模型加载完成后，按住 F8，等提示“录音中”再说话；松开后显示结果；Esc 退出。默认每条最长 180 秒，达到上限会自动停止；这种情况的报告不记为实际松键延迟。验证器与桌面版都默认使用 F8，试录时请先退出桌面版，避免两个程序同时响应。验证入口不修改剪贴板，不向其他应用自动输入。

录音期间默认临时静音开录时的 Windows 默认播放设备，松键、取消或录音异常时恢复原静音状态，在识别/整理前恢复。它只改变静音标记，不改音量，原来已静音的设备仍保持静音；录音过程中才打开的视频也受该设备静音约束。若要保留播放，使用 `--no-mute-playback`。静音失败仍可录音，终端会提示；恢复失败会提示手动恢复，失败细节写入报告。输出设备在录音中途切换后，新设备不在本次保护范围内；强杀进程或断电无法保证恢复。这是播放控制，不是人声分离。

建议先录三种长度：约 15 秒、约 1 分钟、约 2～3 分钟。使用自然中英混说，包含产品名、数字、否定和自我纠正。示例：“请把 Python script 接到 Claude API，timeout 改成12秒，不是20秒；日志保存为JSON，不要上传录音。”这只是测试内容示例，不是模型已能正确识别的承诺。

没有配置密钥时仍可录音，整理显示 `missing_anthropic_api_key` 并保留原始转录；不会偷偷改用其他服务。录音文件、原文、终稿、API usage、阶段耗时和采样资源均保存在项目本地。云端仅发送转录文字与固定整理说明。

录音报告包含实际输入流解析出的设备编号、名称、音频接口和播放静音/恢复结果；取消或录音失败的尝试也记录恢复状态，但不保存取消录音的 PCM/WAV。

## 指标口径

- 模型构建时间、静音预热时间分别记录；这不是重启操作系统后的磁盘冷启动测量。
- `asr.wall_seconds`：本地识别，长录音含本地 VAD 分段。
- `cleanup.elapsed_seconds`：完整云端响应或失败回退的等待，非首 token 时间。
- `release_to_preview_seconds`：F8 释放被检测到之后，包含音频关闭、播放静音恢复、拼接、保存、ASR 和整理，直到最终正文准备好；不含自动粘贴。
- `processing_seconds`：文件基准测试的离线处理耗时。不能当作实际松键或落字延迟。
- 内存为当前测试进程 RSS 的采样峰值，20ms 采样，终端与摘要取 ASR 和整理两阶段的最大值；不包含未持续监测的录音和空闲阶段。CPU 为进程 CPU 时间除以墙钟时间，再按逻辑处理器数量归一化，100% 表示全机逻辑处理器满载。
- GPU 数值是整张卡的快照，包含其他应用；不能归因于本验证进程。ASR 显式使用 CPU provider。
- 参考转录已知时可算混合 token 错误率：中文逐字、英文逐词、数字单独计数，忽略大小写及大部分标点。它不是直接可比较的标准 CER/WER，也不能代替专名、数字、否定与原意的人工检查。
- 小样本分位数带 `low_sample_count` 标志；三次重复不足以形成可靠日常 P95。

整理失败、超时、输出为空、拒答、输出截断、数字被改动或检测到新增常见说明前言（`unexpected_preface`）时保留原文，记录原因；不自动重试付费调用。前言检查是启发式，不能保证发现所有额外解释或语义改写；用户原文已有同样措辞时容许标点整理。提示已强调保留人称和语气，但仍需人工检查。超时后的迟到结果被忽略，但云端已接受的调用仍可能计费。默认整理等待上限 12 秒，长语音可用 `--timeout 30` 调整。

## 命令入口

在项目目录的终端中运行：

```powershell
# 官方短样本及人工拼接的 15/60/180 秒压力测试；完全本地
.venv\Scripts\python.exe -m validation.run bench --mode off --repeat 3 --stress-seconds 15 60 180

# 使用本机录音；reference.txt 是逐字人工参考，不应是 AI 总结
.venv\Scripts\python.exe -m validation.run bench --mode claude --wav "录音.wav" --reference "reference.txt" --repeat 1

# 本地整理的切换接口；需先自行启动兼容服务，模型与质量未验证
.venv\Scripts\python.exe -m validation.run record --mode local

# 只识别；查看设备；手动执行本工具录音缓存清理
.venv\Scripts\python.exe -m validation.run record --mode off
.venv\Scripts\python.exe -m validation.run devices
.venv\Scripts\python.exe -m validation.run cleanup

# 录音时保留播放；适合明确要测试外放干扰的情况
.venv\Scripts\python.exe -m validation.run record --mode claude --no-mute-playback
```

WAV 入口只支持未压缩 16-bit PCM；长录音分段要求 16kHz。`--device 编号` 可选择输入设备。超过 20 秒默认采用 Silero VAD，单段目标上限 15 秒；较短录音先保持整段识别。`--segmentation whole` 只用于重现未分段的对照，长录音可能漏词。

验证器的分段发生在停止录音之后，尚未实现边说边识别；因而几分钟音频仍会把全部识别工作留到结束。VAD 切段可能影响边界词和上下文，评估时应检查这些位置。

## 复现环境

依赖精确版本在 `requirements-validation.txt`。在项目根目录创建独立环境并准备模型：

```powershell
py -m venv .venv
.venv\Scripts\python.exe -m pip install -r requirements-validation.txt
.venv\Scripts\python.exe scripts/download_validation_model.py
```

模型包的来源 URL、实际 SHA-256 和文件列表保存于模型目录 `download.json`；SHA-256 为下载记录，不是独立核验的上游签名。

解压后的 SenseVoice INT8 权重为239,233,841字节，约152MB是压缩包体积。当前桌面版附带的权重和词表与本验证器使用的文件逐字节相同；推理运行时、分段与后处理仍不同，不能将验证器的准确率、耗时或资源占用直接当作桌面版结果。

验证器仅清理 `recordings/validation/` 中七天前的 WAV，不递归删除其他目录；退出后到下次启动前不执行清理。报告保存在 `outputs/validation/`，包含转录正文、参考稿、设备信息和测量数据，由使用者自行管理和删除。这些目录以及 `.env.local` 均应保持 Git 忽略，不随源码发布。桌面版独立维护自己的音频、文字历史和每日清理策略。

## 会议录制基础（开发中）

会议基础检查独立于上述口述验证器，当前不代表完整会议转录功能已通过验收或发布。准备桌面开发依赖及一次本地 Rust 构建后，可以运行：

```powershell
python scripts/check-meeting-store.py
python scripts/check-meeting-capture.py
python scripts/check-meeting-alignment.py
```

默认仅使用合成输入，不打开麦克风、不播放声音、不调用识别模型。采音检查必须显式使用 `--list` 或 `--capture`；`--capture` 限定两个确切设备和最多30秒，不保存音频。可选 `--timings` 仅保存时钟、格式和标记供合成静音重放，不包含音频样本或设备标识。

对齐检查分别验证采样位置完整性、输入时间戳的不确定性、拟合时钟的一致性和起止边界。固定窗口共识容许有界的稀疏时间戳波动；未知缺音、时间戳错误标记、持续时钟跳变仍须失败。合成真值误差和真实设备元数据兼容检查须分别报告，后者不能证明声学相位精度。

`check-meeting-store.py --build-soak` 另构建有界写盘长测程序。`--microphone-channels 2` 可覆盖双声道麦克风；默认麦克风单声道、播放双声道。必须区分两小时音频样本的加速写入与实际经过两小时的持续测试。队列满须显式中断并保留可恢复数据，不能用扩大队列或静默丢包代替稳定性验收。

当前开发分支的受控持续写盘检查已实际运行 7200 秒，保存 1,440,000 个合成数据包、2880 个分块，恢复后逐包核对样本字节、时钟、序号和标记全部一致。输入为 48kHz Float32，麦克风单声道、播放双声道，包含静音包及播放时钟偏移；队列容量仍为 2048 包及 16MiB，实测峰值 196 包。完整运行和校验共 7249.27 秒，进程峰值内存约 7.64MB。

这只证明上述受控输入通过生产队列和存储链的持续性与数据完整性，不替代真实设备连续采音、会议软件兼容、回声处理或转录质量验收。此前一次持续长测在 6749 秒因队列满中断，后续修复了逐包目录检查开销；该失败仍作为故障证据保留。完整外放会议功能尚未通过验收或发布。
