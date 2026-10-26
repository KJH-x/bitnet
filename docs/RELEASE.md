# 构建、签名与分发

构建脚本只生成文件，不安装包、不注册服务、不发布到公网。CI 覆盖 Windows x86/x64/ARM64，Ubuntu amd64/arm64、Debian 12 i386，以及 macOS Intel/Apple Silicon。CI 定义已提供，首次远程执行及各系统安装验收仍待完成。

## Windows 工具与构建

需要现成 Visual Studio C++ Build Tools、Windows SDK、Rust。`scripts/bootstrap-tools.ps1` 将校验过的 .NET SDK 9.0.300、NSIS 3.13、Minisign 0.12 解压到忽略的 `.local/tools`，不会全局安装；已有正确工具时不重复下载。支持 `BITNET_WIX`、`BITNET_MINISIGN`、`BITNET_MAKENSIS` 指定已配置工具。

```powershell
.\scripts\bootstrap-tools.ps1
.\scripts\build-wix-source.ps1
.\scripts\package-windows.ps1 -Arch x64 -Development
```

WiX 固定源码 v7.0.0、提交 `b8977d6f88e7b68e000bac226a2814f236770570`。构建脚本在隔离副本中用源项目引用替换已发布的 WiX 依赖，编译 DUtil、cabinet 处理器与 MSI authorer；不构建 Burn。它不会调用上游会清理用户全局 NuGet 缓存的清理脚本。

该自编译副本按 MS-RL 使用源码；上游 `OSMFEULA.txt` 第 4 条明确允许自行编译且不受该协议约束，因此本地衍生 authorer 移除官方二进制的 EULA 检查。配置外部官方工具时保留其许可与授权要求，脚本不代表用户接受协议。不要将本地 authorer 当官方 WiX 发布，也不要把构建工具混入 BitNet 安装包。参考 [WiX 源码许可](https://github.com/wixtoolset/wix/blob/v7.0.0/LICENSE.TXT)、[自编译条款](https://github.com/wixtoolset/wix/blob/v7.0.0/OSMFEULA.txt)。

当前本机 toolset 为 v145；Visual Studio 2022 可传 `-PlatformToolset v143`。源工具版本、SDK 与压缩工具均固定，但构建尚不承诺逐字节可重现。

`-Development` 在 `.local/release-keys` 生成仅供验收的测试密钥，限制目录 ACL，不输出私钥。禁止将该密钥或开发签名包当正式发布。正式构建使用外部提供的 `BITNET_RELEASE_SECRET_KEY_FILE`、`BITNET_RELEASE_PUBLIC_KEY_FILE`，不加 Development；私钥不进入仓库、安装包或日志。CI 可从受保护 secrets 临时写入这些路径，完成后销毁 runner。`.cargo/config.toml` 将静态 CRT 限定于 Windows MSVC 目标；打包会审查四个程序的导入表，若仍依赖独立 VCRUNTIME/MSVCP/UCRT 则失败。

打包先把公钥编译进四个程序，构建 Service MSI，再对最终 MSI 原始字节签名，最后把 MSI 与 `.minisig` 嵌入 Client MSI，封装 NSIS。没有二进制自包含哈希循环。`-SkipBuild` 仅复用已带同一公钥的产物，脚本会检查维护程序中是否存在该公钥。

## Linux 与 macOS

Linux 在对应最低发行版原生构建 `cargo build --release --workspace --bins`，再执行 `DEVELOPMENT=1 ARCH=amd64 sh scripts/package-linux.sh`。架构取包管理器命名 `amd64/arm64/i386`；版本通过 `VERSION`，二进制路径通过 `BIN_DIR` 覆盖。Python 标准库按 Debian ar/tar 格式生成 core/gui DEB，显式设置 root 所有权和程序/数据权限，再用 `dpkg-deb --info` 验证，兼容 WSL 的 NTFS stage，不运行维护脚本。需准备 OpenSSL、X11、Wayland、xkbcommon、fontconfig 的开发依赖。包依赖写入实际宿主 glibc 最低版本，不得把 Ubuntu 24.04 的产物宣传为 22.04 兼容。

Linux/macOS 构建前先执行 `sh scripts/prepare-signing.sh --development`，再导出 `.local/release-keys/public.env` 中的公钥。正式构建改用外部密钥文件，不使用 development。打包时开发构建需显式 `DEVELOPMENT=1`；脚本会校验维护二进制已编译相同公钥。构建前后须生成与验证源码凭据，打包脚本拒绝缺失凭据、源码变化或标签版本不符的产物：

```sh
sh scripts/prepare-signing.sh --development
set -a; . .local/release-keys/public.env; set +a
python3 scripts/fingerprint-source.py --write .local/build-source-before.json
cargo build --release --workspace --bins
python3 scripts/fingerprint-source.py --check .local/build-source-before.json --write target/release/bitnet-build-source.json
DEVELOPMENT=1 sh scripts/package-linux.sh
```

macOS 最后一行改为 `DEVELOPMENT=1 sh scripts/package-macos.sh`。使用自定义 Cargo target 目录时，相应调整凭据输出位置及 `BIN_DIR`。

macOS 同样原生构建，并额外生成 platform 的 `bitkeychain`。执行 `DEVELOPMENT=1 sh scripts/package-macos.sh` 生成 DMG+PKG。`CODESIGN_IDENTITY` 与 `INSTALLER_SIGN_IDENTITY` 分别控制可选程序与安装包签名。当前按免费分发约束保留项目 Minisign、自持签名与用户 Gatekeeper 手动允许路径，不承诺已获 Apple 信誉。Developer ID、公证和 stapling 是未来可选渠道；脚本不会绕过 Gatekeeper。

Linux 使用项目 `build/` 内唯一 stage；macOS stage 已存在时拒绝覆盖。不会递归删除计算路径。打包调用许可收集器，在各包和个人压缩包中保留项目 LICENSE、gateway 原始 NOTICE、依赖原许可全文和 JSON 审计记录；Linux 静态 OpenSSL 还包含库本身的原始版权及 Apache 协议。

## 发布目录与更新清单

`scripts/source-archive.ps1` 从已提交的 HEAD 生成源码压缩包；当前未提交开发快照可显式使用 `-WorkingTree`，只枚举未被 Git 忽略的文件并使用单独快照名称。应用构建前后对 Cargo、项目编译配置和 crates 内容计算源指纹，变化时拒绝封包；`build-info.json` 或 `build-source.json` 保留对应指纹。

`release-manifest.py` 接受 JSON 数组 spec，每项包含 `path/platform/arch/kind`，可用 `filename` 指定发布时唯一文件名。清单仅放自动更新包，不放 Setup EXE、DMG、完整组合 PKG 或源码等人工下载条目。使用 canonical 架构 `x86/x86_64/aarch64`；Windows kind 为 `client-msi/service-msi`，Linux 为 `core-deb/gui-deb`，macOS 为 `client-pkg/service-pkg`。多个架构的 Service.msi 发布前必须用不同文件名，客户端缓存仍固定为 `packages/Service.msi`。

```sh
python scripts/release-manifest.py --spec release-assets.json --version 0.1.0 --base-url https://downloads.example.test/0.1.0 --out dist/publish
```

上述域名仅说明参数格式。生成器复制实际资产，计算真实字节数与 SHA-256，并生成 schema_version/api_version/version/assets。每资产含 platform/arch/kind/filename/bytes/sha256/urls，须与 [更新协议](UPDATES.md) 一致。签名后不要改动 manifest 的空白、编码或换行。

```powershell
.\scripts\sign-release.ps1 -Directory .\dist\publish -SecretKey <外部私钥路径> -PublicKey <公钥路径>
```

`manifest.json.minisig` 和每个安装包的 `.minisig` 必须与原字节一起发布。SHA256SUMS 便于人工核验，更新信任根仍为编译公钥。更换信任公钥需要先通过可信渠道分发含新公钥的客户端。

分发可使用 GitHub Releases 供人工下载，并用自有域名提供固定 HTTPS 更新清单及安装包。更新客户端拒绝重定向，GitHub 常见下载跳转不能直接作为自动更新源；Pages 放小清单、R2 自定义域名放大文件是合适的静态方案。配置最终 feed、允许的镜像 origin 与公钥后重新编译，当前没有部署任何站点或桶。

## 现有证据与待验收

本机 Windows x64 已实际编译、构建 NSIS+双 MSI+免安装 ZIP，并通过 Minisign 自校验；只读 MSI 表确认 Client 为个人安装、Service 的 ALLUSERS=1、两个 UpgradeCode 独立、服务账户为 NT SERVICE\BitNet、启动参数 --service、数据目录为保护 DACL。四个程序的真实导入表均无 VCRUNTIME、MSVCP 或 api-ms-win-crt 额外运行库。Windows 平台 8 项测试通过，macOS x86_64 平台全部目标已交叉 check/clippy；交叉检查不代表真实 macOS 运行验收。

WSL Ubuntu 24.04 amd64 已构建整个工作区 release，生成并签名 core/gui DEB 和个人 tar.gz。两平台构建源码指纹均为 `aec928fcb7bcd8e8e7a874bdcb76552c1e84392ecaf77077dcdbf374f205a90c`。只读解出 MSI、ZIP、DEB、tar 中的程序、许可文本、离线服务缓存，与 release 目标逐字节比对通过；DEB 中程序为 root:root/0755、许可与模板为 root:root/0644。Windows 的依赖声明收录 277 个包、198 份独立原文；Linux 收录 342 个包、242 份独立原文（含静态 OpenSSL），均启用严格检查且无缺失项。更新清单和源码归档脚本另行验证，不代表已发布公网。

仍须在可销毁环境验证首次安装、取消 UAC、离线启用、修复、升级、保留数据卸载、服务身份 DPAPI 跨重启、恢复动作、Linux 249 DynamicUser 凭据交付、i386运行、macOS Keychain/launchd/签名，以及无校园认证副作用的长时间稳定性。已有构建产物不代表这些用例已通过。
