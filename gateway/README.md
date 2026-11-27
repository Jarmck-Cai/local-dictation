# 轻述轻量听写与词库同步网关（`local-dictation-gateway`）

面向 1–2 vCPU、1–2 GB 内存 Linux VPS（支持 `x86_64` 与 Ampere `aarch64`）的纯 Rust 单二进制网关服务：

- **客户端零上游密钥**：`OPENAI_API_KEY` 与 `ANTHROPIC_API_KEY` 仅保存在 VPS `/etc/local-dictation-gateway/gateway.env`（`0600`），桌面端与笔记本仅持有分设备签发的 `Bearer` 访问令牌，可按设备独立吊销。
- **实时识别与旧版兼容**：普通听写可边录边由网关转发给 OpenAI `gpt-live-transcribe`。整段上传继续使用 `/v1/audio/transcriptions`（默认 `gpt-transcribe`）。网关在内存中处理音频。
- **两级流水线与安全排版**：一级声学识别注入高频技术词表；二级调用 Anthropic Claude Haiku（默认 `claude-haiku-5-5`）完成去口水词、口误消解、口述“第一/第二/第三”转跨行 `1. / 2. / 3.` 有序列表及中英同音术语纠偏，内置数值/正负号/否定词防篡改护栏，二级超时或异常自动优雅降级返回 `raw_text`。
- **多端 SQLite 词库同步**：内置 `rusqlite`（`bundled`，零宿主机动态库依赖）维护用户已确认词条、手动词与高频/点赞技术词，支持跨设备去重合并与 `since_revision` 增量同步。

## 接口概览

| 方法与路径 | 鉴权 | 说明 |
| --- | --- | --- |
| `GET /healthz` | 无 | 健康与版本探针（无敏感信息） |
| `GET /v1/status` | `Bearer <device_token>` | 返回设备标识、上游布尔就绪态、模型名与当前词表版本号 |
| `GET /v1/vocabulary?since_revision=N` | `Bearer <device_token>` | 拉取版本号 `> N` 的增量/全量同步词表 |
| `POST /v1/vocabulary/sync` | `Bearer <device_token>` | 上传本机词条并合并去重，返回服务端最新增量 |
| `POST /v1/dictate` | `Bearer <device_token>` | `multipart/form-data` 上传内存音频与元数据，返回 `raw_text`、`formatted_text`、`cleanup_applied`、`fallback_reason`、`asr_ms`、`cleanup_ms` |
| `GET /v1/dictate/stream` | `Bearer <device_token>` | WebSocket 接收实时音频，结束录制后返回与整段接口相同的终稿结构 |

### 实时听写

网关 0.2.0 默认通过 `OPENAI_REALTIME_ASR_MODEL=gpt-live-transcribe` 启用实时识别。将该变量显式设为空值可保留整段上传服务并停用实时入口。客户端会在实时路径不可用时退回整段上传。代理到网关的线路须支持 WebSocket 升级。

客户端连通后先发送与整段接口相同的元数据 JSON，再发送 24 kHz 单声道小端 PCM16 二进制帧。松键时送完余下音频，再发 `{"type":"finish"}`。暂定文字可由 `partial` 事件返回，最后返回 `DictateResponse`。取消录音时关闭该录音的 WebSocket。网关向 OpenAI 显式提交录音末尾，再沿用整段文字整理与回退策略。 [OpenAI 实时转录协议](https://developers.openai.com/api/docs/guides/realtime-transcription)

## 本机测试与跨架构静态编译

编译与测试建议将 `CARGO_TARGET_DIR`、`TEMP`、`TMP` 指向大容量磁盘目录：

```powershell
$env:CARGO_TARGET_DIR = "E:\Projects\local-dictation\.cache\gateway-target"
$env:TEMP = "E:\Projects\local-dictation\.cache\tmp"
$env:TMP = "E:\Projects\local-dictation\.cache\tmp"
cargo test --manifest-path gateway/Cargo.toml
```

面向 1–2 GB 小内存 VPS 请勿在远端编译，在本机使用 `cargo zigbuild` 交叉编译纯静态二进制：

```powershell
# x86_64 Linux VPS
cargo zigbuild --manifest-path gateway/Cargo.toml --release --target x86_64-unknown-linux-musl

# Oracle Ampere aarch64 Linux VPS
cargo zigbuild --manifest-path gateway/Cargo.toml --release --target aarch64-unknown-linux-musl
```

## 一键安全部署

详见 `gateway/deploy/`：

- `local-dictation-gateway.service`：带内存上限与文件系统隔离保护的 `systemd` 服务单元
- `Caddyfile.example` / `nginx-dictation-gateway.conf`：HTTPS / 本地反向代理配置示例（亦支持直接挂载 Cloudflare Zero Trust Tunnel `http://127.0.0.1:8787`）
- `gateway.env.example`：服务端环境变量模板
- `deploy-oracle-vps.ps1`：从本机 `.env.local` 与 Windows DPAPI 安全读取密钥并经 SSH 加密标准输入写入远端 `/etc/local-dictation-gateway/gateway.env`（`0600`），全程不在终端或日志打印任何密钥字符
