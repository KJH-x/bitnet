# BitNet

北京理工大学校园网认证与连接管理器。Rust 核心、独立 Iced 软件渲染 GUI、CLI 与系统后台共用同一认证逻辑。

**当前为开发版本。** 各平台的实现进度见 [实现与验证状态](docs/STATUS.md)。

公开分发入口为 [GitHub Releases](https://github.com/KJH-x/bitnet/releases)。桌面端可在连接页仅登录本次，也可验证并保存账户。Windows、macOS 和支持 StatusNotifierItem 的 Linux 桌面可用 `--silent` 启动至托盘。

## 使用方式

- `bitnet-gui`：桌面界面，提供连接状态、账户管理、后台服务和软件设置。网卡、网络规则及日志在窗口内打开。
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
bitctl diagnose --output report.json
bitctl service status
```

默认不开启自动认证。首次配置后选择开启。手动注销会持久暂停自动认证，用户主动连接或恢复后解除。外网或 DNS 异常不触发注销；密码被明确拒绝后停止针对该凭据的自动登录。

开启自动认证前需确认当前网络属于校园连接。未知网络的手动登录也需要明确确认；账户、网关或网络变化后仍会重新评估。确认记录不是 HTTP 网关的加密身份证明。

## 开发与安全演示

需要 Rust 1.88 或更新工具链和各平台原生编译工具；用户安装包不需要 Rust、Python、Node 或 WebView。

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
- [200 项需求覆盖矩阵](docs/FEATURE-MATRIX.md)
- [依赖许可与来源](docs/LICENSES.md)

代码采用 MIT 许可证。校园网客户端的上游跟进目标是 [BITERNET（bitoolkit/biternet）](https://github.com/bitoolkit/biternet)，原名 BitGateway，协议库现名 `biternet-client`。本项目的 SRUN 编解码移植自迁移前的 `bitgateway-client`，原始提交、版权和许可见 [gateway NOTICE](crates/bitnet-gateway/NOTICE)。`KJH-x/BIT-Connect` 与本地 netdiag 用作需求和设计参考。
