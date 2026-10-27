# 源码来源

本目录为 Local Dictation Windows 内测版的工作副本，基于 [Handy v0.9.8](https://github.com/cjpais/Handy/releases/tag/v0.9.8)，MIT 许可见同目录 LICENSE。

- 上游 commit：`14f6f0d31cb22a4268acfbddba3052820fd0a06e`
- 官方归档：`https://codeload.github.com/cjpais/Handy/zip/14f6f0d31cb22a4268acfbddba3052820fd0a06e`
- 下载日期：2026-10-26
- 归档 SHA256：`4007e060e2b2c45af47baf5eb43670c128fb19c7648ff396086b1dd07abbee41`
- 原归档与未修改展开副本：项目忽略的 `.cache/upstream/`，不依赖它们运行。

本项目拥有独立产品名称、数据目录和更新策略。保留部分 `handy` 内部 Rust 符号以缩小迁移范围；这些内部符号不代表上游官方发行版。

现有 Python 验证工具与数据位于项目根目录，未打包进桌面运行时。配套权重和词表经逐字节核对与原验证文件相同；运行时、分段及后处理不同，性能不能互相代替。固定资源见 RESOURCES.md。

## 轻述品牌资源

0.1.6 起的应用及托盘图标为本项目原创几何标志，源文件为 `../icons/qingshu.svg`；`../../../scripts/export-qingshu-icons.py` 从同一源导出 PNG、ICO 和 ICNS，保留16/24px的光学校准。导出工具使用 Pillow，仅用于开发，运行程序不需要它。保留原有资源文件名以兼容安装结构，上游许可和来源说明仍然保留。
