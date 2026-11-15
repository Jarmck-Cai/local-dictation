# 会议回声消除试验交接

`v0.1.12-beta.4`（`d794568`）是这次归档的产品基线。接续日常听写或会议功能开发时，从 `origin/codex/meeting-mode-012` 或该标签起步。需要复跑或比较回声消除时，使用 `codex/meeting-aec-research`。这个研究分支归档了 SpeexDSP 原型，桌面应用的构建和会议处理入口均未接入它。

## 保留的材料

- [`aec.rs`](../desktop/src-tauri/src/meetings/aec.rs) 和 [`aec_tests.rs`](../desktop/src-tauri/src/meetings/aec_tests.rs) 保存 Rust 封装与单元测试。输入为同一时基上已对齐的 16 kHz 单声道数据，每帧 160 个采样点。同一录制段持续使用一个滤波状态。双设备的对齐和时钟漂移校正由调用方负责。
- [`build_meeting_aec.rs`](../desktop/src-tauri/build_meeting_aec.rs) 保存静态编译辅助代码。两套测试脚本各自构建独立的原生库和 Rust 测试程序，无需编译桌面应用。
- [SpeexDSP 1.2.1 源码子集](../desktop/src-tauri/native/speexdsp-1.2.1/README.md) 保留上游 `COPYING`、`AUTHORS` 和原始文件声明，包括 SmallFT 的版权和许可。若今后随产品分发其二进制，随包附上这些声明。

## 如何复跑

在 Windows x64 上使用已装 NumPy 的 Python、MSVC x64 C 工具、面向 MSVC 的 `rustc` 和 eSpeak NG。从仓库根目录按顺序运行，生成的音频及报告留在 Git 忽略的 `outputs/022-meeting/` 下。eSpeak NG 的安装路径与默认值不同时，两个命令均可传 `--espeak <程序路径>`。

```powershell
python scripts/check-meeting-aec.py --diagnostic --extended
python scripts/check-meeting-aec-v2.py --extended
```

第二条命令读取第一条命令写入默认目录的 `aec-report.json`，记录其摘要。两条命令均预期退出 `1`，对应已知的声学门槛失败。用 v1 报告的 `pass=false` 和 v2 报告的 `complete=true`、`challenge_pass=false`、`pass=false` 确认脚本已完成。音频由 eSpeak 写到文件，测试过程无需麦克风或播放设备。

## 已知结果与继续验证的边界

- **V1 原始口径**直接对比未处理的近端波形。12 个已对齐基线场景的远端单讲抑制约 24–26 dB，但近端波形相关性约 0.972，低于 0.99，双讲残差降低约 7.4–7.7 dB，低于 10 dB。40 ms 延迟且未纠正 ±100 ppm 漂移时，远端单讲抑制约 4.2 dB。
- **V2 独立参考**按上游公开的直流陷波公式计算近端参考。20 个基础场景、14 个额外稳态场景及 7 个 Rust 单元测试通过。声路突变后立即双讲仍失败，3 秒后残差降低约 9.22 dB，低于 10 dB；完整 v2 验收仍为失败。
- **后续实用验证**重点观察外放时远端声音是否被归到麦克风轨，以及交接时紧跟的短回复、否定词和数字是否保留。历史试验表明，直接按远端语音活动窗口切麦克风可能吞掉正常接话。比较候选方案时应保留双轨原音和同一套测试条件；真实房间、设备长录及转录语义仍需单独取证。

产品侧的发布状态和已确认边界见 [版本记录](releases.md)。
