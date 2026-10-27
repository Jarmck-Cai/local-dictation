# iPhone 原型 · 轻听写

目标设备为 iPhone 16，最低 iOS 18。手机使用 SenseVoiceSmall INT8 离线识别中英混说，录音停止后才识别。**当前是开发原型；CI 编译通过不代表完成真机准确率、速度、续航或跨应用验收。** Windows 产品不受此目录影响。

## 在 Mac 安装

需要完整 Xcode 16.4 或更新版本，并在 Xcode → Settings → Accounts 登录 Apple ID。用数据线连接 iPhone、信任电脑，并按系统提示启用开发者模式。

```sh
git clone --branch codex/ios-prototype https://github.com/Jarmck-Cai/local-dictation.git
cd local-dictation
brew install xcodegen
bash ios/scripts/prepare.sh
open ios/LocalDictation.xcodeproj
```

在 Xcode 选择以下方案，再到对应目标的 Signing & Capabilities 选择自己的 Team。若 Bundle Identifier 已被占用，在本机 `Config/Developer.xcconfig` 修改 `APP_BUNDLE_PREFIX` 为自己独有的值，主 App 和扩展会一起更新。选择连接的 iPhone，点击 Run。

| 方案 | 签名条件 | 能验证的内容 |
| --- | --- | --- |
| **LocalDictationOffline**（普通 Apple ID 首选） | Personal Team；无 App Group | 离线混说、屏幕按钮、动作按钮快捷指令、录音实时活动、应用内验证框自动追加文字 |
| LocalDictation | 支持 App Groups 的开发团队 | 上述功能，以及轻量键盘跨应用自动落字 |

普通 Apple ID 的安装有系统规定的有效期，过期后需通过 Xcode 重新签名。完整版本需主 App、键盘和实时活动目标使用同一 Team；`Config/Developer.xcconfig` 中的 `APP_GROUP_ID` 要与三个目标实际注册的 App Group 一致。免费签名不能靠关闭能力或伪造共享目录获得跨应用访问。

## 第一次使用

1. 在 App 内点击“下载并校验离线模型”，等待约 242 MB 权重、词表及 VAD 下载完成。下载地址、版本、大小和 SHA-256 固定；之后识别不需要网络。
2. 点击“开始听写”，允许麦克风。再次点击停止，或点击实时活动中的停止。最长 120 秒，系统中断也会先释放麦克风。
3. 普通 Apple ID 版本把结果自动追加到本页验证框，可以继续编辑或复制。文字只进入验证框，不会发送消息。
4. 在系统设置 → 动作按钮 → 快捷指令，选择“开始或停止轻听写”。辅助触控的小白点可配置运行同一快捷指令。首次启动需要 App 进入前台，再按一次停止；这不是电脑端按下/松开事件，不能劫持任意音量/侧键组合。

完整版本另需设置 → 通用 → 键盘 → 添加“轻听写”并允许完全访问，用于通过本机 App Group 共享任务。目标输入框先切换到轻听写键盘，再按动作按钮开始录音；停止后**手动返回原应用**。键盘确认同一文档和光标上下文时插入一次，否则点“插入本次文字”。键盘不能直接使用麦克风，不能借助公开 API 自动切回任意第三方应用；密码框、电话键盘及禁止第三方键盘的应用不支持。原型没有系统级悬浮球，辅助触控由 iOS 提供。

## 自动文字增强

默认离线。用户点击一次“允许，以后自动判断”后，较长或有口头语的文字才尝试增强；短句直接使用本机结果。客户端最多等待约两秒，超时、取消、鉴权失败、保护校验不通过时使用离线原文。已落字结果不会被晚到的增强覆盖；关闭开关会取消在途请求并使尚未领取的增强结果回到原文。数字、英文词和部分否定词有保守守卫，但不能保证语义绝对一致，发送前仍需确认。

后端源码见 [services/ios-enhancement](../services/ios-enhancement/README.md)。**仓库和 CI 默认构建没有可用增强服务配置**，不会产生模型调用。部署后由开发者把 HTTPS `/v1/enhance` 地址和受限原型客户端令牌放进本机 `Config/Service.plist`（从 example 复制），重新打包。用户不输入 API；供应商密钥只放服务端。客户端令牌可从安装包提取，仅用于受限内测，正式发布前应替换为用户/设备鉴权及可撤销配额。

只上传本次识别文字，不上传录音，不抓取剪贴板或输入框正文。输入框的本机指纹包含系统文档 ID 与光标前后文字的 SHA-256，只用于确认落字位置，不发送到后端。App Group 中保留最近一次文字与任务状态；没有个人历史同步。录音在 App 私有目录，打开应用时清理超过一天的文件，也可在 App 一键清除。处理失败保留录音供主动重新识别。模型、录音、共享任务不参与 iCloud 备份。

## 真机验证清单

| 场景 | 应观察到 |
| --- | --- |
| 模型下载后开启飞行模式，混说中文、英文缩写和产品名 | 仍能产出文字；人工核对遗漏、专名和数字 |
| 屏幕按钮/动作按钮开始，再停止 | 录音指示消失后才显示“处理”；记录停止到文字出现的时间 |
| 120 秒、来电/系统中断、取消 | 麦克风释放；无重复落字；失败可主动恢复本机录音 |
| 普通 Apple ID 版本连续两次听写 | 本页每次只追加一次，可复制/编辑 |
| 完整版同一输入框、空输入框、移动光标/变更选区/切换文档 | 同一上下文自动插入；变化时要求手动插入；不会自动重试重复输入 |
| 关闭增强/断网/服务超时、处理中撤销同意 | 无需 API 设置；使用离线文字；晚到结果不覆盖 |
| 杀掉 App 再打开 | 未完成任务显示中断，用户主动恢复；不自动复播旧结果 |

`insertText` 没有系统级成功回执：键盘先持久领取再插入，宁可在极端崩溃后让用户回 App 复制，也不自动重插。后台处理时间由 iOS 决定；耗时较长时需打开 App 重新识别。模型初次加载、长音频、耗电、发热和中英混说质量都需在 iPhone 16 实测，当前无性能承诺。

## 开发验证

```sh
swift test --package-path ios/LocalDictationCore
bash ios/scripts/prepare.sh
xcodebuild -project ios/LocalDictation.xcodeproj -scheme LocalDictationOffline \
  -destination 'generic/platform=iOS' CODE_SIGNING_ALLOWED=NO build
```

macOS CI 分别测试纯策略，编译模拟器、完整 iPhone App/键盘/实时活动与普通签名版本，并检查服务配置确实被打包。CI 附件未签名，不能直接安装到手机。来源和权重独立许可见 [THIRD_PARTY.md](THIRD_PARTY.md)。
