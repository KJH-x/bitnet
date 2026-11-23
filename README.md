# BitNet

北京理工大学校园网认证与连接管理器。Rust 引擎为桌面界面、终端界面、命令行和系统后台提供同一套认证逻辑。

**当前为开发版本。** 各平台的实现进度见 [实现与验证状态](docs/STATUS.md)。

公开分发入口为 [GitHub Releases](https://github.com/KJH-x/bitnet/releases)。桌面端可在连接页仅登录本次，也可验证并保存账户。Windows、macOS 和支持 StatusNotifierItem 的 Linux 桌面可用 `--silent` 启动至托盘。

## 使用方式

- `bitnet-gui`：桌面界面，侧栏包含连接、账户、网络、后台服务和软件设置。连接区提供状态、详情、统计和诊断。详细日志通过「软件设置 → 日志」在浏览器中打开。Windows 可以双击托盘图标切换窗口显示。
- `bitnet-tui`：在当前终端查看状态和日志、管理认证。可用 `bitctl tui` 启动，按 `Tab` 切换状态与日志，按 `p` 切换 IP 地址显示。IP 地址默认隐藏。
- `bitnet-web`：按需启动的本机日志查看器，可筛选、分页、查看详情及导出 JSON。用 `bitctl web start`、`bitctl web stop` 或软件设置控制。
- `bitctl`：交互式与自动化管理。密码通过隐藏输入或标准输入提交，不接受命令行密码参数。
- `bitengine`：独立后台，用户模式或系统服务模式。GUI 退出不必终止认证引擎。
- `bitmaint`：安装、修复、启动项和更新维护。系统操作按需授权，不常驻。

```text
bitctl configure
bitctl status --json
bitctl account add --id campus --username YOUR_ACCOUNT
bitctl connect
bitctl disconnect --yes
bitctl resume
bitctl config set automatic true
bitctl trust-network --yes
bitctl profile save --id campus --name Campus
bitctl history --limit 100
bitctl tui
bitctl web start
bitctl web stop
bitctl diagnose --output report.json
bitctl service status
```

默认不开启自动认证。首次配置后选择开启。手动注销会持久暂停自动认证，用户主动连接或恢复后解除。外网或 DNS 异常不触发注销；密码被明确拒绝后停止针对该凭据的自动登录。

开启自动认证前需确认当前网络属于校园连接。未知网络的手动登录也需要明确确认；账户、网关或网络变化后仍会重新评估。确认记录不是 HTTP 网关的加密身份证明。

## 开发与安全演示

构建桌面端和完整工作区需要 Rust 1.93 或更新工具链，以及平台原生编译工具。桌面端的滚轮动画由固定到上游提交的 Iced 0.15 开发版提供。用户安装包不需要 Rust、Python、Node 或 WebView。

```text
cargo test --workspace
cargo build --workspace
cargo run -p bitnet-cli -- --help
```

Windows PowerShell 的隔离演示（合成账户，无校园网请求）：

```powershell
$env:BITNET_DATA_DIR = Join-Path $PWD '.local/demo'
$env:BITNET_SIMULATE_ONLY = '1'
New-Item -ItemType Directory -Path $env:BITNET_DATA_DIR -Force | Out-Null
target/debug/bitctl.exe --data-dir $env:BITNET_DATA_DIR start --simulate
target/debug/bitctl.exe --data-dir $env:BITNET_DATA_DIR account select demo
target/debug/bitctl.exe --data-dir $env:BITNET_DATA_DIR connect
target/debug/bitctl.exe --data-dir $env:BITNET_DATA_DIR status --json
target/debug/bitctl.exe --data-dir $env:BITNET_DATA_DIR stop
```

演示模式的账户仅保存在内存；设置和暂停状态保存在指定演示目录。不要在演示中输入真实校园网密码。开发测试不会安装系统服务、修改网卡、终止其他程序或执行真实注销。

## 架构、安装与发布

- [架构与安全边界](docs/ARCHITECTURE.md)
- [安装与静默部署](docs/INSTALL.md)
- [发布与签名](docs/RELEASE.md)
- [CLI 与 GUI 的统一接口](docs/CONTRACT.md)
- [原始需求](docs/REQUIREMENTS.original.zh-CN.md)
- [实现与验证状态](docs/STATUS.md)
- [桌面布局、终端和日志查看器](docs/FRONTENDS.md)
- [200 项需求覆盖矩阵](docs/FEATURE-MATRIX.md)
- [依赖许可与来源](docs/LICENSES.md)

代码采用 MIT 许可证。校园网客户端的上游跟进目标是 [BITERNET（bitoolkit/biternet）](https://github.com/bitoolkit/biternet)，原名 BitGateway，协议库现名 `biternet-client`。本项目的 SRUN 编解码移植自迁移前的 `bitgateway-client`，原始提交、版权和许可见 [gateway NOTICE](crates/bitnet-gateway/NOTICE)。`KJH-x/BIT-Connect` 与本地 netdiag 用作需求和设计参考。
## GUI 自动化

桌面程序内置 [iced-mcp](https://github.com/allient/iced-mcp) 桥接。运行 `bitnet-gui --iced-mcp` 前，设置 `BITNET_SIMULATE_ONLY=1`、独立测试目录的绝对路径 `BITNET_DATA_DIR` 和至少 32 字符的随机 `ICED_MCP_TOKEN`。可用 `ICED_MCP_PORT` 指定空闲端口，再用 `iced-mcp-server` 和同一令牌连接 `127.0.0.1`。普通启动不监听端口。桥接不提供系统剪贴板或应用消息日志。
