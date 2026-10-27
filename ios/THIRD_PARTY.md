# iOS 原型的第三方资源

代码依赖和模型权重分别遵守其上游许可。模型按首次使用时的下载操作获取，不随此源码仓库分发。原型的运行时、SHA 校验和编译通过不代表已完成 iPhone 16 的准确率、速度或能耗实测。

| 资源 | 固定版本 / 来源 | 许可 |
| --- | --- | --- |
| sherpa-onnx Swift / iOS runtime | [v2.0.1](https://github.com/k2-fsa/sherpa-onnx/tree/v2.0.1)，由官方 SwiftPM 提供静态 XCFramework | [Apache-2.0](https://github.com/k2-fsa/sherpa-onnx/blob/v2.0.1/LICENSE) |
| ONNX Runtime | sherpa-onnx 的官方 SwiftPM 依赖 [onnxruntime-libs 1.28.2](https://github.com/csukuangfj/onnxruntime-libs/tree/v1.28.2) | [MIT](https://github.com/microsoft/onnxruntime/blob/main/LICENSE)；保留依赖附带的 notices |
| SenseVoiceSmall INT8 ONNX 与词表 | [官方 sherpa-onnx 作者的转换](https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/tree/2365baeacb507f821a0c8120fcee3d484dba7a07)；固定提交 `2365baeacb507f821a0c8120fcee3d484dba7a07` | [该导出的 LICENSE](https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/blob/2365baeacb507f821a0c8120fcee3d484dba7a07/LICENSE) 指向 FunASR 模型许可；见下文 |
| Silero VAD v4 ONNX | [Handy 固定资源](https://blob.handy.computer/silero_vad_v4.onnx)，来源 [Silero VAD](https://github.com/snakers4/silero-vad) | [MIT](https://github.com/snakers4/silero-vad/blob/master/LICENSE) |

SenseVoiceSmall 由 FunAudioLLM / FunASR 发布。[官方权重卡](https://huggingface.co/FunAudioLLM/SenseVoiceSmall) 标注独立的模型许可；[官方说明](https://github.com/QwenAudio/SenseVoice#license) 明确区分代码许可与权重许可，链接到 [FunASR Model Open Source License Agreement](https://github.com/modelscope/FunASR/blob/main/MODEL_LICENSE)。请勿将 sherpa-onnx 的 Apache-2.0 或上游工具代码的 MIT 视为模型权重许可。当前原型保留 SenseVoiceSmall 模型名称和来源；正式分发产品时仍需按模型许可保留完整协议、署名和产品中的模型标识。

下载后逐文件验证 SHA-256，大小与摘要见 `App/Offline/ModelManager.swift`：

| 文件 | 字节 | SHA-256 |
| --- | ---: | --- |
| `model.int8.onnx` | 239233841 | `c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51` |
| `tokens.txt` | 315894 | `f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc` |
| `silero_vad_v4.onnx` | 1807522 | `a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28` |

词表与 VAD 的上述大小、摘要已实际下载核对；INT8 摘要由固定 HF 提交的 LFS 元数据和本项目既有桌面资源校验记录交叉核对，本轮没有为测速而重新下载大模型或运行真人音频。
