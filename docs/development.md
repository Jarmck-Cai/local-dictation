# 开发与版本管理

公开仓库：<https://github.com/Jarmck-Cai/local-dictation>。`main` 保存已完成相关检查的代码，功能分支使用 `codex/` 前缀。

## 每次开发

1. 开始前检查分支、工作区和远端状态，保留已有未提交工作；读取 `AGENTS.md` 及变更目录下的项目约定。
2. 将改动拆成可审查的功能或修复。完成一个有意义的阶段后，运行对应检查，审阅差异及暂存文件，再使用 Conventional Commits 提交并推送当前分支。不必等到正式发布才备份进度。
3. 用户已授权日常提交与推送，无需逐次询问。合并已有协作改动时先核对远端；默认不重写远端历史、不强制推送。若检查失败或发布受阻，说明实际状态，不把未完成的工作称为已发布。

纯文档改动核对链接和内容；程序改动执行相应离线测试、前端检查或 Rust 检查。正式桌面交付需构建并核对产物。已有检查通过后，仅在新改动或未解决问题需要时扩大验证；真人试录、基准或付费 API 调用按已确认范围执行。

手动补贴的离线回归入口为 `python scripts/check-manual-paste.py` 和 `python scripts/check-protected-paste.py`，需要 Rust 编译器。检查使用生产状态与策略代码及合成数据，不操作系统剪贴板、不发出键盘输入、不调用模型。它们覆盖单次手势、忙时拒绝、按键释放、绑定升级及冲突、剪贴板恢复与并发复制；不能据此宣称目标应用已成功落字。原生构建和应用兼容性验证另行记录。

自动输入回归使用 `python scripts/check-automatic-paste.py`，执行真实编排与共享事务的合成慢读、连续正文、取消和并发复制检查；`--legacy` 应复现旧机制的失败，不能作为新版通过命令。元数据兼容检查为 `python scripts/check-dictation-metadata.py`，读取实际Rust类型及已构建debug缓存的serde依赖，使用合成JSON。品牌矢量源为 `desktop/src-tauri/icons/qingshu.svg`，`scripts/export-qingshu-icons.py` 使用开发环境Pillow导出应用/托盘资源；应用运行不新增依赖。

按键状态回归使用 `python scripts/check-keyboard-state.py`，以合成原始输入和状态核对扩展键残留、真实按下、配置绑定、未知基线、设备/会话变化及同步代次；默认不监听本机键盘、不注册原始输入、不发键、不操作剪贴板。错误提示接线由 `python scripts/check-overlay-wiring.py` 核对，覆盖自动错误码、原生通知、前端白名单与中英文文案。原生观察器的注册和生命周期检查另行明确执行，不能用类型编译或合成状态代替实际API检查。

会议占用录制时的普通听写回归使用 `python scripts/check-meeting-busy-feedback.py`，提取生产协调状态、共享录制许可和悬浮通知状态运行合成检查，覆盖拒绝时无录制副作用、重复按键与释放、结束会议后不自行开始，以及新按键恢复。该入口不使用麦克风、播放声音或发出系统按键；原生提示窗口及完整会议功能需另外验收。

## 正式交付

Windows快照兼容回归使用 `python scripts/check-win32-snapshot.py`，默认只运行合成Win32适配器，不接触系统剪贴板。`--baseline`用于旧版失败对照。显式原生只备份诊断需额外传 `--native-snapshot --windows-rlib <已构建windows-0.61.3.rlib>`：仅在进程内读取/备份并释放，输出静态状态和格式数，不写剪贴板、不发键、不输出或保存正文；不能将其称为实际输入验证。`--build-native-only`只编译该诊断。

Windows发布边界回归使用 `python scripts/check-win32-publication.py`，提取实际适配器方法，合成CloseClipboard的系统文本格式转换，核对发布后许可与恢复；默认不调用系统剪贴板。`--baseline`读取v0.1.7，应复现相同断言失败。这与只备份检查覆盖不同边界，不能互相替代，也不能代替完整原生流程或目标应用验收。

该检查的 `--rdp` 模式增加纯合成的远程剪贴板时序，并使用生产单次按键控制器，分别建模格式列表传播、本机正文请求和远端编辑器消费；覆盖预读、延后读取、取消、并发复制、恢复与不重复注入。通过表示实现符合这些特征断言，不表示真实 RDP 已落字。`--rdp --rdp-delivery-gate` 额外要求两个已知失败模型收到本次正文，当前应以两个断言失败退出，用于保留显式红灯；不作为默认成功检查。模型假设、官方依据与待实测步骤见 `docs/remote-dictation-plan.md`。

`check-protected-paste.py --native-text-memory --windows-rlib <已构建windows-0.61.3.rlib>` 只执行生产 UTF-16 内存构造与固定代码页转换，不访问剪贴板、窗口、键盘或用户正文。`--build-manual-rdp-probe <输出exe>`（同样需 `--windows-rlib`）只构建手动对照诊断，不自动运行。其实际模式会替换当前剪贴板且不能恢复测试前原内容，必须先明确固定文本、受控编辑器和用户测试窗口；不发键、不读原正文，新复制优先，按固定基线结算。详见远程输入调查记录，不能用构建或内存检查代替真实落字验收。

用户可见功能或修复形成交付时，统一更新 `desktop/package.json`、`desktop/src-tauri/Cargo.toml`、`Cargo.lock` 中本包版本及 `tauri.conf.json`，并更新 README 和 `docs/releases.md`。检查通过后提交，建立不可变的 `vX.Y.Z` 标签，并推送提交与标签。后续修复使用新版本号。

公开安装包只作为 GitHub Release 附件，源码 Git 不包含安装包、模型或运行数据。附件须核对版本、SHA-256、第三方资源许可及不含用户数据。

## 公开边界

Git 仅保存源码、合成测试、依赖锁文件、公开文档和必要许可证。密钥、`.env.local`、录音、历史数据库、学习样本、凭据、本机缓存、安装目录、输出日志及个人交接记录留在本机，由 `.gitignore` 排除。提交前审阅新增文件；公开历史中的旧版本也要遵守同一边界。

保留 Handy 的 MIT 许可、`desktop/UPSTREAM.md` 和资源来源。模型等第三方资源采用各自许可，代码许可不替代资源许可。

## 本次历史导入

此前本机没有 Git 提交。本次使用现存备份导入可核对的旧版源码，再提交当前版本；提交时间为实际导入时间，不伪造开发历史。可还原范围见 `docs/releases.md`。私人交接与试录分析不在公开仓库中。
