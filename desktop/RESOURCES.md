# 固定运行资源

2026-10-26 下载并校验；`scripts/prepare-desktop.ps1` 可恢复被忽略的构建资源。

| 资源 | 官方来源 | SHA256 |
| --- | --- | --- |
| Bun 1.4.4 Windows x64 zip | https://github.com/oven-sh/bun/releases/tag/bun-v1.4.4 | `cd2c0fccc78f3ede1f50ca827a15f063eda13d4d150a3d0674f4f6e323d864a5` |
| ONNX Runtime 1.24.2 Windows x64 zip | https://github.com/microsoft/onnxruntime/releases/tag/v1.24.2 | `8e3e9c826375352e29cb2614fe44f3d7a4b0ff7b8028ad7a456af9d949a7e8b0` |
| Silero VAD v4 ONNX | https://blob.handy.computer/silero_vad_v4.onnx | `a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28` |
| SenseVoice INT8 tar.gz | https://blob.handy.computer/sense-voice-int8.tar.gz | `171d611fe5d353a50bbb741b6f3ef42559b1565685684e9aa888ef563ba3e8a4` |
| 解压的 model.int8.onnx，239233841 bytes | 同上 | `c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51` |
| 解压的 tokens.txt | 同上 | `f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc` |

配套 SenseVoice 权重与词表经本机逐字节核对，均与早先 Python/sherpa 验证所用文件一致。此前方案以压缩包152MB与权重239MB误判不同导出的文字已废止。推理运行时、VAD策略和后处理仍不同，不能把旧性能测量当作桌面程序的保证；本次仅校验文件，没有新运行识别。

原生 ONNX 使用四线程共享池、一线程跨算子调度、关闭空闲自旋及遥测。Windows 构建关闭 Vulkan 和动态 Whisper 后端，仅展示 SenseVoice。

## 本地中文分词

极简词表使用 [jieba-rs 0.11.0](https://github.com/messense/jieba-rs/tree/v0.11.0) 的默认内置词典进行本地中文分词，不启用 TF-IDF 或 TextRank。代码及词典随依赖版本固定，校验值见 Cargo.lock；MIT 许可随应用保留在 `resources/jieba-rs-LICENSE.txt`。分词仅用于词表收集，不调用网络、不参与自动替换或云端提示，也不保证发现所有专名或判断识别正确性。