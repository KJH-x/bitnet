# MyGo 能否让 BitNet 桌面端更轻

2026-11-11。调研对象为 [egoist/mygo v0.8.3](https://github.com/egoist/mygo/releases/tag/v0.8.3)。结论：**保留 Iced 作为主界面；若将来允许 64 位专用方案，优先对照实测 MyGo 的原生 UI，而非以 WebView 模式作为减负方案。** 本轮为文档和源码调研，没有运行原型、改动界面或重新打包。

| 项目 | 现有 Iced | MyGo WebView | MyGo 原生 UI |
| --- | --- | --- | --- |
| 渲染 | tiny-skia 软件渲染，不用 WebView | 借用系统网页引擎，安装包不捆绑浏览器 | MyGo 用 Go 自绘，可走 Metal、D3D11、OpenGL 或 CPU；不启动 WebView |
| Windows | 当前 x64 产物无需 WebView；x86/ARM64 属于目标矩阵 | Windows 10/11 x64、ARM64，依赖 WebView2；缺失时需另装 | x64、ARM64，避开 WebView2 |
| Linux | 当前发行路径使用 X11/Wayland 等桌面库，目标含 Debian i386 | x64、ARM64，GTK 3 + WebKitGTK 4.1 或 4.0 | x64、ARM64，仍需 GTK 3 |
| macOS | Intel/Apple Silicon 在目标矩阵，实机验收待完成 | macOS 12+，WKWebView 随系统提供 | macOS 12+，无需 WebView |

来源：[BitNet 架构](ARCHITECTURE.md)、[安装基线](INSTALL.md)、[MyGo v0.8.3 平台要求](https://github.com/egoist/mygo/blob/v0.8.3/docs/README.md#requirements)、[渲染及平台实现](https://github.com/egoist/mygo/blob/v0.8.3/docs/architecture.md#goals-and-constraints)。该版本明确说明 32 位 Windows/Linux 即使可以编译，也会在启动时使用不支持的平台后端并报错，原生模式同样受限。其 Linux 托盘还需要 AppIndicator 库。构建方面无需 cgo，但该标签的 [go.mod](https://github.com/egoist/mygo/blob/v0.8.3/go.mod) 指定 Go 1.27.3，官方 Web 模板使用 Bun；这是开发工具链，用户端仍只需应用与对应系统运行库。

## 区分三个体积指标

- **下载体积**：本地 2026-10-31 Windows x64 `bitnet-gui.exe` 为 4,871,680 字节（4.65 MiB）；完整安装器为 6,285,959 字节（6.00 MiB）。[本地二进制和产物记录](validation-2026-10-31.json)。上游架构文档称 MyGo 示例程序约 7 MB，未显示出有把握的减包空间。示例的系统、界面功能、压缩方法不同，不能直接判定谁的安装包更大。
- **首次安装占用**：MyGo WebView 可复用已有系统引擎；Windows 缺少 WebView2 时需联网引导安装，或离线提供完整运行时。应同时计算“已有运行时”和“干净离线系统”两种成本。[微软 WebView2 部署说明](https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution)
- **运行开销**：MyGo 架构文档给出 macOS 样例约 62 MB（含 WebView 相关进程），另一个小型原生窗口约 44 MB。上游独立的[原生 UI 基准](https://mygo.egoist.dev/benchmarks)在 M5、MyGo 0.7.4 上给出小应用 9.7 MB、首帧 173 ms、静置 34.1 MB；其对照是 GPUI 和 egui，不含 Iced。样例与测量口径不同，数字不应互减当作 WebView 的固定开销。Windows WebView2 应计入浏览器、渲染器等关联进程。[MyGo 架构数据](https://github.com/egoist/mygo/blob/v0.8.3/docs/architecture.md#goals-and-constraints) · [微软进程模型](https://learn.microsoft.com/microsoft-edge/webview2/concepts/process-model)。BitNet 现有 11.99 MiB 记录来自认证引擎，不是 Iced GUI 内存。

## 对 BitNet 的建议

1. 保留目前的 Rust 引擎、CLI、安装和更新链。MyGo 即使可用于界面，也不必替换它们。网页模式的优势主要在于复用 HTML/CSS、图表和布局；本项目的加密诊断计划只在用户端展示摘要，并不因此需要引入 WebView。
2. 若重点是降低实际常驻开销，先测量并减少 Iced 窗口隐藏后的定时刷新（目前每 2 秒有状态读取节拍），保留托盘所需更新；只需联网的用户可只运行独立引擎。界面优化和平台替换是两个独立决定。
3. 如果以后对照试验，只做同一套状态页、托盘和日志表，用相同模拟数据、刷新频率、窗口大小与缩放。记录完整安装器与额外运行时、冷启动至可操作的中位/P95、可见/隐藏/退出三种状态的 CPU 和内存、日志滚动与高 DPI。WebView 要统计关联进程，区分私有内存和共享页。再验离线安装、中文输入、托盘与无障碍，保留 32 位目标作为迁移门槛。
4. 如试验 Go 前端，通过现有本机 IPC 做窄的客户端适配，保留对端身份校验与凭据边界。若采用网页模式，只给打包的应用页面开放必要方法；诊断 HTML、外部页面和任意本地文件不得共享认证桥。[MyGo 页面信任规则](https://github.com/egoist/mygo/blob/v0.8.3/docs/architecture.md#trust)
