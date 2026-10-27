# iOS 文字增强网关原型

离线识别在 iPhone 完成。网关只接收识别后的文字，通过 Anthropic Messages API 做保守整理；不接收录音，不记录文字、令牌或上游错误正文。用户不需要配置 API，服务管理员配置一次后将 HTTPS 地址和受限客户端令牌打包进内测应用。

当前交付为源码和合成测试，尚未部署。没有可用公网增强地址时，手机应保持离线回退。管理员的部署和提供商配置独立于用户界面。

## 接口

```http
POST /v1/enhance
Authorization: Bearer <prototype-client-token>
Content-Type: application/json

{"requestId":"550e8400-e29b-41d4-a716-446655440000","text":"今天用 Swift 写 3 个函数，不要修改 API。"}
```

成功返回同一 `requestId` 和整理后的 `text`，上游失败时返回原文。请求必须严格只有这两个字段，最多 8,000 个 Unicode 码点、64 KiB 请求体。`GET /health` 只报告进程可达，不表示提供商配置可用。其他接口、方法、音频类型和模型覆盖字段均拒绝。

上游总期限 1.6 秒，包含读取返回正文；手机应独立执行 2 秒总期限、撤销和过期任务检查。网关不重试。已取消的请求不开始上游调用；已经到达提供商的调用即使取消也可能计费。

数字、URL、英文和代码 token、部分中文否定/条件/确定程度词不能改变；拒绝前言、代码围栏、截断或长度异常结果。守卫不能证明语义完全一致，也不能识别所有中文专名。原始文字仍应在手机上可取回。

## 验证

使用 Node 22 或更新版本，无 npm 运行依赖：

```sh
cd services/ios-enhancement
npm test
```

测试使用合成文字与本地提供商替身，不联网、不产生模型费用。包含严格输入、鉴权、硬超时、取消、错误保密、内容守卫、全局预算、并发与重启持久化。Windows 受限沙箱可能禁止原子文件替换；应在正常用户终端或 CI 运行完整测试。

## 推荐部署：Cloudflare Worker

Worker + 一个 SQLite Durable Object 就能运行，不需要 VPS。所有节点共享一个原型预算：滚动一分钟最多 6 次、滚动 24 小时最多 100 次及 100,000 输入字符，并在该窗口内拒绝同一请求 ID。只有 UUID、时间和字符数会持久保存。服务重启和切换节点不能重置预算。预算存储不可用时拒绝增强。

这只是内测访问控制。应用内客户端令牌可以被提取；发布前应加入每设备认证、轮换/吊销与提供商账单限额。不要把它当成提供商 API 密钥，也不要用无限额度的提供商凭据。代码中的次数和字符限制不等于货币账单上限。

管理员在自己的终端完成一次 Cloudflare 授权：

```sh
npx --yes wrangler@4.153.0 login
cp wrangler.toml wrangler.private.toml
```

在被 Git 忽略的 `wrangler.private.toml` 加入自己的 `account_id`，以及独立子域名，例如：

```toml
account_id = "YOUR_ACCOUNT_ID"

[[routes]]
pattern = "dictation-api.example.com"
custom_domain = true
```

部署时确认名称未被占用，保留 `workers_dev = false`、`preview_urls = false`、SQLite migration 和 `BUDGET` 绑定。不要复用或更改其他站点。

先部署关闭状态的服务，再输入服务器 secrets：

```sh
npx --yes wrangler@4.153.0 deploy --config wrangler.private.toml
npx --yes wrangler@4.153.0 secret put ANTHROPIC_API_KEY --config wrangler.private.toml
npx --yes wrangler@4.153.0 secret put ANTHROPIC_MODEL --config wrangler.private.toml
npx --yes wrangler@4.153.0 secret put CLIENT_TOKEN --config wrangler.private.toml
```

`ANTHROPIC_MODEL` 由管理员选择当前账户可用的便宜模型，代码没有猜测或硬编码模型 ID。`CLIENT_TOKEN` 使用至少 32 字符的密码学随机值；保存到本地受控配置，再通过开发者构建配置传给 iOS 主应用。提供商密钥始终只在服务器，不能打包进应用或提交到 Git。不要用 CI 日志打印这些值。

部署后先用 `/health` 和带鉴权的无效请求检查 DNS、TLS、鉴权与输入拒绝，均不需要付费模型调用。提供商实际联通和端到端延迟应在获授权的设备验收中单独确认。Wrangler 当前要求 Node 22+；CLI 登录只授权部署，iPhone 不需要 Cloudflare 账户或设置界面。

## VPS 备用适配器

`node-server.mjs` 复用相同网关，固定只监听 `127.0.0.1:3107`，由反向代理负责 HTTPS。配置通过服务端环境提供：`CLIENT_TOKEN`、`ANTHROPIC_API_KEY`、`ANTHROPIC_MODEL`、`BUDGET_FILE`。启动命令为 `npm start`。

`BUDGET_FILE` 指向专用私有目录中的文件，使用事务串行化和 fsync + 原子替换持久化预算；损坏时关闭增强，不静默清空。此适配器只支持单进程，不用于集群或多实例。运行用户只能访问自己的代码、环境和预算文件。应设置内存上限、限制反向代理请求体、禁止缓存和正文日志，并把 3107 留在回环接口。

## 官方资料

- [Anthropic Messages API](https://platform.claude.com/docs/en/api/messages/create)
- [Anthropic API 版本](https://platform.claude.com/docs/en/api/versioning)
- [Cloudflare SQLite Durable Objects](https://developers.cloudflare.com/durable-objects/)
- [Durable Objects migrations](https://developers.cloudflare.com/durable-objects/reference/durable-objects-migrations/)
- [Wrangler 登录与凭据](https://developers.cloudflare.com/workers/wrangler/commands/general/)
