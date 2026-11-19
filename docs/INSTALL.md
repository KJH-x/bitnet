# 安装与维护

当前产物使用开发签名密钥，供验收。Windows x64 已实际构建双 MSI、NSIS 和免安装 ZIP；Ubuntu 24.04 amd64 已构建 DEB 与个人压缩包，并验证项目签名。没有在开发者主机安装、注册服务或执行校园网认证。Ubuntu 22.04、其他架构及 macOS 的实际支持仍以 CI、隔离虚拟机和真机验收为准。

## Windows

普通用户运行 `BitNetSetup-<版本>-windows-<架构>.exe`。默认安装桌面界面、CLI、TUI、Web 日志查看器、个人引擎和维护工具到 `%LOCALAPPDATA%\Programs\BitNet`。选择后台服务时才请求系统授权；取消授权会保留已成功安装的客户端。完成页可直接启动界面。个人数据位于 `%LOCALAPPDATA%\BitNet`，与程序文件分开。程序采用静态 CRT，不要求另装 VC++ 运行库。

命令行程序未写入全局 PATH。可在 PowerShell 使用 `& "$env:LOCALAPPDATA\Programs\BitNet\bitctl.exe" status`，或进入安装目录运行 `.\bitnet-tui.exe`。免安装 ZIP 提供同一组程序、许可文本及已签名的离线服务缓存。默认数据仍在 AppData。需要独立数据目录时，在启动前设置 `BITNET_DATA_DIR` 为绝对路径，只影响个人模式。

无人值守服务独立安装到 Program Files，固定服务名 `BitNet`，身份为 `NT SERVICE\BitNet`，系统数据位于 `%ProgramData%\BitNet`。凭据用该身份的用户范围 DPAPI 加密，数据 ACL 只允许服务身份、SYSTEM 与 Administrators。客户端和服务具有独立 MSI ProductCode、UpgradeCode 与文件组件；卸载程序不删除账号数据。

静默安装客户端：

```powershell
.\BitNetSetup-0.1.0-windows-x64.exe /S /MODE=user
```

静默安装客户端及后台服务使用 `/S /MODE=service`，调用者须已有管理员权限；静默模式不会弹出授权框。非静默安装也可稍后在界面启用服务。标准退出码 0 表示成功，3010 表示需重启；服务步骤失败时返回失败码，客户端仍保留。

`Client.msi` 内缓存 `packages/Service.msi` 和 `Service.msi.minisig`，离线可启用服务。维护工具用编译公钥验证缓存，提权后复制到管理员保护的暂存目录再次验证，再调用系统 MSI。不可用、损坏或缺少签名时拒绝安装。

客户端修复可在 Windows 已安装应用中选择修复，或执行 `msiexec /fa Client.msi /qn /norestart`。后台维护统一使用：

```powershell
bitmaint --scope system --action repair --interactive
bitmaint --scope system --action service-remove --interactive
```

卸载默认保留数据；显式 `--purge` 才请求删除对应应用数据。客户端完整卸载会在当前用户上下文删除固定两项 BitNet 启动值；重大版本升级保留这些值及用户设置。直接 MSI 默认阻止降级，只有经过验签的显式回退维护计划传入固定 `BITNET_ALLOW_DOWNGRADE=1`。Windows Installer 自带的恢复动作表存在上游已知限制；正常 NSIS/GUI 维护路径另以 SCM 原生接口设置恢复动作，直接安装 Service MSI 的实际恢复行为仍需虚拟机验收。

## Linux

基线为 Ubuntu 22.04 amd64/arm64（systemd 249），32 位基线为 Debian 12 i386（systemd 252）。32 位 Rust 目标要求 i686/SSE2，不承诺更早 CPU。`bitnet-core` 提供后台、CLI 与维护工具。`bitnet-gui` 包提供桌面界面、TUI 和 Web 日志查看器，依赖同版本 core。

```sh
sudo apt install ./bitnet-core_<版本>_<架构>.deb ./bitnet-gui_<版本>_<架构>.deb
```

core 安装时仅在主密钥不存在时生成 32 字节 `/etc/bitnet/masterkey`，root:root、0600；修复与升级保留原密钥。`bitnet.service` 使用 DynamicUser、StateDirectory 和 LoadCredential，不创建普通登录账号。引擎低权限运行，主密钥由 systemd 交付；账号库用 AES-256-GCM 加密。完整磁盘及主密钥同时被复制、root 权限被控制不在该保护范围内。

systemd 创建 `/run/bitnet` 为 0755；引擎创建 `control.sock` 为 0666，并按对端身份授权命令。目录不可由普通用户写入。此处没有 socket activation，避免与引擎重复绑定。

界面中的服务移除只停用服务与活动 unit，保留程序包、可信模板和账号数据以便重新启用；完整删除系统包仍需 `apt remove/purge`。DEB 的 purge 默认也保留账号密文及主密钥，只有明确的应用数据清理动作才删除对应数据。

Ubuntu 24.04 等较新系统可由管理员将主密钥转为 `systemd-creds --with-key=host encrypt` 输出，并通过 drop-in 替换为 `LoadCredentialEncrypted=masterkey:<加密文件>`；当前默认包保留 systemd 249 兼容方式，不自动迁移或丢弃旧密钥。该方式不依赖 TPM。

无需管理员的个人模式使用 `bitnet-user-*.tar.gz`。解压后可在同一目录运行 `./bitnet-gui`、`./bitnet-tui` 或 `./bitctl`。账号加密主密钥放入桌面 Secret Service，需要 `libsecret-tools` 和已解锁的提供程序，例如 GNOME Keyring 或启用 Secret Service 的 KeePassXC。它不能代替无人登录时的系统服务模式；缺少密钥环时明确报错，绝不降级明文。

## 其他安装方式

- Windows 的 `BitNetPortable-<版本>-windows-<架构>.exe` 双击运行，在当前用户的本地应用数据目录展开独立引擎与桌面程序。支持 `--silent`。需要离线系统服务时使用安装器或完整 ZIP。
- Linux AppImage 解压到用户私有数据目录的版本子目录，保障 GUI 退出后个人引擎和启动项仍可用。WSL 或无 FUSE 环境可使用 `--appimage-extract-and-run`。用 `--cli`、`--tui` 或 `--web` 进入对应程序，例如 `./BitNet.AppImage --cli status`。
- Linux RPM 将 GUI、CLI、TUI、Web 日志查看器与个人引擎安装至系统路径，需要 Fedora 等 RPM 发行版的桌面依赖和 Secret Service 提供程序。系统服务仍通过支持该路径的 DEB 分发。
- Cargo 安装面向具备平台编译依赖的个人模式。将所需程序安装到同一个 Cargo bin 目录。

```sh
cargo install --locked --git https://github.com/KJH-x/bitnet bitnet-runtime --bins
cargo install --locked --git https://github.com/KJH-x/bitnet bitnet-cli
cargo install --locked --git https://github.com/KJH-x/bitnet bitnet-gui
cargo install --locked --git https://github.com/KJH-x/bitnet bitnet-tui
cargo install --locked --git https://github.com/KJH-x/bitnet bitnet-web
```

Linux x86_64 和 aarch64 用户可执行 `nix run github:KJH-x/bitnet`。运行 TUI 或 Web 日志查看器时追加 `#tui` 或 `#web`。NixOS 或 Home Manager 的 flake 可分别导入 `inputs.bitnet.nixosModules.default` 或 `inputs.bitnet.homeManagerModules.default`。模块管理个人图形会话：

```nix
services.bitnet = {
  enable = true;
  autoStart = true;
  silentStart = true;
};
```

## macOS

DMG 内可拖拽 `BitNet.app` 运行个人模式。CLI、TUI 和 Web 日志查看器位于 App 的 `Contents/MacOS` 目录。需要开机无人值守时安装 DMG 内 Service PKG，安装器请求管理员授权。App 自带同一签名 Service.pkg 缓存；首装缺少维护 helper 时界面打开系统安装器，并提示完成安装后刷新，打开安装器不等于安装成功。Client.pkg 仅更新 App，Service.pkg 仅更新后台，个人模式更新不会顺带开启服务；完整组合 PKG 供明确选择完整安装时使用。

PKG 安装隐藏、禁止登录的 `_bitnet` 系统身份，后台 worker 使用该身份。按需 root Keychain broker 使用 launchd Sockets 激活，只有 root 或 `_bitnet` 可取得应用主密钥；普通用户仅通过管理 IPC 修改共享账号，不能取回密码。broker 每次启动先准备开机后重建的 `/var/run/bitnet`，再访问 System.keychain。维护模板与 helper 位于 root 控制的 `/Library/PrivilegedHelperTools`。

macOS Keychain 无交互解锁、应用签名 ACL、broker 启动、升级后访问和重启尚未真机验收。当前按免费分发约束使用项目 Minisign，可选自持代码签名；首次运行可能需要在 macOS 隐私与安全性中手动允许，Minisign 不会自动获得 Gatekeeper 信誉。构建成功不等于无人值守验证通过。Developer ID 与公证只是未来可选渠道。

## 安全恢复边界

密钥或凭据损坏时，账号密文保留，界面应提示重新录入或修复。服务已安装但不可用时不能悄悄启用个人引擎争抢认证。账号库、配置与程序升级分别受权限控制。共享管理 IPC 不返回已保存密码。

`BITNET_DATA_DIR` 只覆盖个人数据位置，可用于便携或隔离测试；系统服务绝不接受此覆盖。测试目录应位于项目 `.local/` 或 `target/`。
