# 轻述 macOS Beta 5 预览

## 同步基线

Windows 最新已发布的桌面预览为 [0.1.12-beta.5](https://github.com/Jarmck-Cai/local-dictation/releases/tag/v0.1.12-beta.5)，桌面源码为 `b053568`，网关为 0.2.0。自 0.1.11 起加入本地双路会议录音、断点恢复、会议列表和详情、MSTSC 输入、网关词库同步，以及边录边识别。0.1.11 已有历史分页、全量搜索、纠正和自动词表。仓库历史最早导入的 0.1.2 源码已包含继承自 Handy 的 macOS 采音、辅助功能和粘贴适配器。同步前没有可核对的独立 macOS 分支或安装包，用户设备上的版本需要在该设备确认。

| 模块 | macOS Beta 5 |
| --- | --- |
| 听写、历史和词库 | 共享 Beta 5 的实时云端、批量及本机回退路径，沿用历史与词库数据库 |
| 凭据 | 在登录钥匙串单独保存 Claude 密钥与设备令牌。升级保留应用标识和数据目录 |
| 自动输入和手动补贴 | 查询按键状态和辅助功能权限。以 NSPasteboard 的读取回执结算一次粘贴，保留后续复制。手动补贴沿用 Ctrl+Shift+F9，实际粘贴使用系统快捷键 |
| 会议 | macOS 15+ 用 ScreenCaptureKit 记录所选麦克风和主显示器对应的系统声音，沿用本地双路存储、处理、导出和 AEC3。可能含其他应用音频，纯音频采集 |
| 安装与登录 | Apple Silicon DMG。macOS 系统数据路径与已有登录项机制。Windows MSTSC 和安装维护任务属于 Windows 专用功能 |

## 构建

Apple Silicon 运行听写需要 macOS 14 或更新版本。构建使用包含 macOS 15 SDK 的 Xcode、Rust stable、Bun 1.4.4、CMake、Meson 和 Ninja。先从仓库根目录准备资源，再从 `desktop` 构建。

```sh
bash scripts/prepare-macos.sh
cd desktop
MACOSX_DEPLOYMENT_TARGET=14.0 CMAKE_POLICY_VERSION_MINIMUM=3.5 \
  bun run tauri build --target aarch64-apple-darwin -- --locked
```

[macOS GitHub Actions](https://github.com/Jarmck-Cai/local-dictation/actions/workflows/macos.yml) 构建并校验 Apple Silicon DMG。产物使用 ad-hoc 签名，面向预览验证。Intel 使用同一份适配源码，ONNX Runtime 1.24.2 的动态库需要单独配置，构建路径可参考 [Handy 的 Intel ONNX 准备步骤](../desktop/.github/workflows/build.yml)。Intel 安装包尚待构建和验收。

## Beta 5 的 Mac 首启更新

`0.1.12-beta.5.macos.1` 在权限缺失时引导授权麦克风及辅助功能，返回应用可重新检测并初始化快捷键。未配置网关时默认用本地 SenseVoice，可稍后在设置中用连接测试启用云端优先。Mac 听写忽略旧配置的播放静音选项。授权、覆盖升级、旧静音配置及云端切换需在真机验证。

## 原生验收

依次检查安装与升级后历史读取、Option+Space 本地听写、云端实时与断网回退、普通应用和浏览器的落字、复制新内容优先、Ctrl+Shift+F9 补贴，以及保存与清除钥匙串中的设备令牌。首次运行授予麦克风和辅助功能权限。macOS 15+ 会议选定麦克风并显式开始后，系统可能要求屏幕与系统音频录制权限。用耳机和外放分别验证双路录音、停止、设备断开、中断恢复、导出和回声消除效果。会议源标签表示录音来源。CI 构建、文件校验和合成回归与这些交互式验收分别记录。
