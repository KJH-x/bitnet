# 安装维护与项目签名更新

维护入口统一为 `bitnet_runtime::maintenance::execute(scope, action, interactive)`；GUI 与 CLI 使用同一个实现。维护命令不接受自定义命令、脚本、路径、下载 URL 或密码。

## 发行前必须配置的信任信息

构建时设置以下环境变量；它们编入二进制，不读取用户运行时环境变量：

| 变量 | 用途 |
| --- | --- |
| `BITNET_RELEASE_PUBLIC_KEY` | Minisign 公钥的 base64 单行，或完整 `.pub` 文本 |
| `BITNET_UPDATE_FEED` | HTTPS manifest 的完整地址，例如自有域名的 `/releases/latest.json` |
| `BITNET_UPDATE_ALLOWED_ORIGINS` | 可选的逗号分隔 HTTPS origin；默认只允许 feed 自身 origin |

没有 feed 或公钥时，更新明确返回未配置；没有公钥时，Windows 离线服务安装也拒绝执行。项目没有内置开发用公钥或虚构更新源。生产签名私钥不放仓库、不放安装包，并应离线备份。更换信任公钥需要发布包含新公钥的可信客户端。

建议 Pages 提供 canonical HTTPS manifest，R2 自定义域名提供无需重定向的包下载。更新器拒绝 HTTP、代理、URL 用户名/密码、fragment，以及所有 HTTP 重定向。GitHub Releases 可作为人工下载镜像；其会跳转的浏览器下载链接不能直接用作当前自动更新地址。公开 HTTPS 文件托管即可，无需在线业务服务。

## 固定维护命令

```
bitmaint --scope system --action service-install --interactive
bitmaint --scope system --action service-start --interactive
bitmaint --scope system --action service-stop --interactive
bitmaint --scope system --action service-remove --interactive [--purge]
bitmaint --scope system --action repair --interactive
bitmaint --scope user --action check-update
bitmaint --scope user --action install-update --interactive
bitmaint --scope user --action rollback-update --interactive
bitmaint --scope user --action autostart-on
bitmaint --scope user --action autostart-off
bitmaint --scope user --action gui-autostart-on
bitmaint --scope user --action gui-autostart-off
```

`--interactive` 允许本次操作请求系统授权；不传时权限不足直接失败。`--elevated` 仅供授权后的固定 helper 使用，要求真实管理员身份。GUI 启动项与个人认证引擎启动项独立，系统服务不依赖 GUI 进程。命令输出 JSON，成功退出码为 0，需要重启为 3010，失败使用共享错误码。

Windows 每次系统维护分别请求 UAC，不创建可执行任意操作的常驻高权限端点。MSI 启动使用系统目录中的 `msiexec.exe`。Linux 使用固定 `/usr/bin/pkexec` 和 `/usr/libexec/bitnet/bitmaint`；macOS 使用固定 `/usr/bin/osascript` 授权调用 `/Library/PrivilegedHelperTools/top.nsapi.bitnet.maint`。AppleScript 内只包含固定 helper 路径、封闭动作名、scope 和可选 purge 标记，没有用户输入的脚本内容。Unix helper 与全部父目录必须 root 持有且不可被 group/other 写入。

Windows“停止后台”先禁用服务再停止，因此也阻止 SCM 已排队的故障恢复重启，并持续保持关闭；“启动后台”恢复自动启动再启动。只需暂停自动认证时使用认证页 Pause。停止后台、退出 GUI、暂停认证是三个不同操作。

## Windows 离线服务安装链

Client MSI 将以下资源一起安装到当前用户的程序目录：

```
bitmaint.exe
packages/Service.msi
packages/Service.msi.minisig
```

Service MSI 的固定 UpgradeCode 为 `{7FBA8135-7341-4F91-A39E-615190032EA2}`；Client UpgradeCode 为 `{81E2553E-095D-49AF-A95A-B0FD47A3E61E}`。服务名为 `BitNet`。

流水线先构建已嵌入公钥的二进制和 Service MSI，再对最终 MSI 原始字节执行 Minisign 签名，最后把 MSI 和 `.minisig` 放进 Client MSI。服务包不包含自己的散列或签名，因此不存在自身散列循环。

安装入口先用编译公钥验证缓存 MSI，然后持有个人数据目录 `maintenance.handoff.lock` 的真实 OS 锁，通过已验证的个人 IPC 请求 Shutdown，并等待 `engine.lock` 释放，再请求 UAC。SDK 的 `maintenance_in_progress()` 检测活锁，防止 GUI 在授权期间重启个人引擎；残留的未锁文件不阻塞启动。Shutdown 保留校园认证会话。取消 UAC 或安装失败后，只有系统服务尚未注册时才恢复原个人引擎；已经注册但未运行的服务要求修复，避免两个控制器争抢全局互斥锁。

授权 helper 再次验签，将 MSI 与签名复制到系统 Known Folder 下 `BitNetMaintenance` 的受保护暂存目录，并重新验证暂存内容后才调用安装器。系统暂存目录只授予 SYSTEM 与 Administrators 完整权限，并核验已有目录所有者；拒绝 reparse point、符号链接、相对路径和路径穿越。每个暂存文件独占新建，有限额，并在安装完成后清理。ServiceInstall/Repair 在 MSI 成功后通过原生 ChangeServiceConfig2W 再次配置失败后重启策略（5/10/30 秒，86400 秒重置，包含非崩溃错误退出），避免仅依赖 Windows Installer failure-action 表。启动和停止动作等待服务状态确认；MSI 报告成功但服务未就绪不会被当作成功。

初次分发的 Setup/Client MSI 本身仍需要用户通过可信渠道取得，并在需要时使用独立可信的 Minisign 工具和单独核对的项目公钥验证。程序自带的公钥不能替代首次下载的信任来源。Windows SmartScreen 与 macOS Gatekeeper 的系统信誉/允许步骤不由项目签名取代。

## Linux 与 macOS 系统维护

Linux 系统包使用固定 systemd unit `bitnet.service`，维护动作仅调用固定参数的 systemctl/dpkg。移除运行 unit 后，重新安装/修复会核验 root 持有的 `/usr/lib/bitnet/templates/bitnet.service` 与所有父目录权限，恢复到 `/etc/systemd/system/bitnet.service`；模板默认保留。macOS 首次从 DMG 复制 App、系统 helper 尚不存在时，GUI 只允许从自身固定 `Contents/Resources/packages/Service.pkg` 读取包，使用编译公钥验证相邻 `.minisig` 后打开系统 Installer。此时结果为 `installer_opened:true, installation_confirmed:false`；用户必须在系统安装器完成授权与安装，再刷新服务状态。打开安装器不代表安装成功，也不作为后续特权动作的授权。已有 helper 时，每次操作仍通过固定 helper 独立请求授权并重新验证在线更新内容。

macOS 使用以下 root 持有的模板，ServiceInstall/Repair 恢复两份 LaunchDaemon plist：

```
/Library/PrivilegedHelperTools/BitNetTemplates/top.nsapi.bitnet.plist
/Library/PrivilegedHelperTools/BitNetTemplates/top.nsapi.bitnet.keychain.plist
```

运行配置位于 `/Library/LaunchDaemons/`；worker 标签 `top.nsapi.bitnet`，按需凭据 broker 标签 `top.nsapi.bitnet.keychain`。先注册 broker socket，再启动 worker。停止 worker 使用 bootout，防止 KeepAlive 立即重启；下次 Start 重新 bootstrap。Remove 卸载两项 job 并移除运行 plist，保留 root 模板、维护 helper 和主密钥以便重装。系统命令失败时返回错误，不把无法查询状态解释为成功。原生 macOS 安装/授权/停止及 Intel/Apple Silicon 运行验证需要在对应系统上完成。

Linux/macOS 在线更新的 DEB/PKG 是系统安装，即使由个人 GUI 发起也必须管理员授权，并使用 root 暂存目录。保留数据是默认行为；`--purge` 只清除固定系统应用数据目录。平台凭据主密钥的生命周期由平台安全存储实现管理。

## 签名 manifest 契约

`latest.json` 的原始 UTF-8 字节签名放在 `latest.json.minisig`；签名后不要格式化或改换行。签名采用 Minisign 默认的 prehashed 模式。示例结构如下，大小与散列必须替换为最终分发文件的真实值：

```json
{
  "schema_version": 1,
  "api_version": 1,
  "version": "0.2.0",
  "assets": [
    {
      "platform": "windows",
      "arch": "x86_64",
      "kind": "client-msi",
      "filename": "Client.msi",
      "bytes": 123456,
      "sha256": "<64 hexadecimal characters>",
      "urls": ["https://downloads.example.test/0.2.0/Client.msi"]
    }
  ]
}
```

支持 platform `windows/linux/macos`、arch `x86/x86_64/aarch64`（macOS 不支持 x86）。Windows 按 scope 选择 `client-msi` 或 `service-msi`；Linux 系统更新选择 `core-deb`；个人 GUI 更新同时选择 `core-deb` 和 `gui-deb`，完成两个包的下载和复验后一次调用 dpkg 安装，保证同版依赖可共同配置；macOS 按 scope 选择 `client-pkg` 或 `service-pkg`，拒绝混合包自动更新；Full.pkg 仅供人工完整安装。对应扩展名必须为 `.msi/.deb/.pkg`。每个 `(platform, arch, kind)` 唯一，未知字段、未知 schema/API 版本、重复条目、非法文件名和不匹配平台格式都会拒绝。每个 asset 可指定 1–3 个可信 origin 上的镜像。任何包都必须有签名保护的字节长度和 SHA-256。

流水线签名示例（私钥路径仅示意）：

```
minisign -S -s <private-key-file> -m Service.msi -x Service.msi.minisig
minisign -S -s <private-key-file> -m latest.json -x latest.json.minisig
```

验证顺序为：HTTPS 获取有限大小元数据 → 验证 manifest 原始字节签名及 trusted comment → 严格解析 schema → 校验稳定 SemVer 必须高于当前编译版本 → 选择平台包 → 流式下载并核验字节数与 SHA-256 → 重新打开受保护暂存文件核验 → 固定系统安装器。较旧、相同版本与预发布版本不能经更新入口替换当前版本；无更新时明确返回没有更新的稳定版本。当前没有独立的“历史上见过的最高 feed 版本”记录，防降级以已安装运行版本为界；它不保证服务器提供的是历史最新 release。

manifest 上限 256 KiB、签名 8 KiB、包 512 MiB、下载限速 6 MiB/s；元数据请求超时 20 秒，包请求 300 秒。失败不执行安装器，不报告安装成功。测试使用项目内合成包、测试密钥和暂存目录，不运行 UAC、真实安装器、宿主启动项或校园认证。

## GUI 自动检查策略

`runtime::update_policy` 由个人 GUI 的现有定时器驱动。启动延迟 30 秒，成功后间隔 24 小时，失败后退避 1 小时；任务运行中不会重复派发。签名网络检查作为独立异步 GUI 任务运行，不进入认证循环或 Status 处理。关闭 `check_updates` 不派发检查；缺少编译 feed/key 时直接返回 `Unconfigured`，没有网络或安装器操作。已验证的相同/较旧 manifest 在被动检查中返回 `NoUpdate`；真正安装入口始终要求更高的稳定版本。

`auto_update` 只允许 Windows 当前个人模式、且系统服务尚未注册时，启动独立的 `bitmaint --scope user --action install-update`，不带交互提权参数。独立 updater 重新下载并验证签名和包，并使用当前用户 MSI 安装。返回 `InstallerStarted` 只表示独立进程已启动，不表示安装完成。系统服务模式无论设置如何都只提示有新版本，必须由用户主动点击更新并单次授权。Linux/macOS 的 DEB/PKG 更新影响系统，因此自动检查也只提示，等待用户主动授权安装。

## 显式恢复上一已验证版本

`RollbackUpdate` 只从本机已保留的前版缓存恢复，不联网寻找旧版本，不自动触发故障回滚。User/System 使用不同缓存索引，按照各自签名资产集合恢复；不能通过选择 User 作用域绕过系统包授权。System 每次单独请求授权，Linux/macOS 的 User 包安装也影响系统并要求授权。

成功更新后保存 manifest 的原始字节、`.minisig` 与实际使用的安装包。下次升级只把缓存中与当前运行版本完全匹配的 active 版本记为 previous；当前编译版本、索引记录版本不匹配时不回退。初始源码/首次安装的版本没有历史签名包，或历史缓存已丢失、被篡改、不完整时，明确返回不可用，不合成旧包或宣称支持回退。

每个作用域最多 active/previous 两个版本；同一维护暂存根下所有作用域的 manifest、签名和资产合计预算为 1 GiB。达到预算不保存新版本，并在安装结果中说明；有条件时保留原 active 作为可恢复前版。失效和多余 generation 只在验证固定缓存根、父子关系及无链接后清理，其他作用域的缓存不被静默驱逐。缓存写入失败不会把已完成的安装误报失败，但结果明确说明回退可能不可用。

回退必须重新验证完整 manifest 签名、相同 API/schema、稳定版本确实低于当前版本、当前平台/架构/作用域、每个安装包字节数及 SHA-256。将包复制到新的受保护暂存目录，再次验证签名元数据和暂存包后才调用固定系统安装器。不会接受任意路径、远程 URL、跨 schema/API 或未签名 fallback。

Windows MSI 默认仍拒绝较旧产品：安装表检测已安装高版本，仅显式回退计划传入固定 `BITNET_ALLOW_DOWNGRADE=1` 时放行。普通更新、安装和修复不传此属性，普通更新仍严格要求更高稳定版本，并拒绝预发布及 build metadata。macOS PKG、Linux DEB 回退沿用平台安装器，Linux GUI+core 在同一次 dpkg 调用中安装；这些平台事务不承诺整个软件数据或系统的原子恢复。回退不迁移账户、密码或历史数据，未来数据格式变更需要另行声明兼容性。

回退继续使用当前构建中的公钥及来源许可配置。发布者变更公钥、来源域名或 API/schema 时，需要明确考虑历史缓存兼容性；不会因为缓存曾经被接受过就跳过当前信任验证。

## 本地验证记录（2026-10-27）

| 环境 | 维护/签名/缓存/HTTP loopback | 更新策略 | bitmaint 参数 | Clippy |
| --- | --- | --- | --- | --- |
| Windows x86_64 MSVC | 26 通过 | 4 通过 | 1 通过 | runtime `--all-targets --no-deps -- -D warnings` 通过 |
| Ubuntu 24.04 / WSL x86_64 | 25 通过 | 4 通过 | 1 通过 | 同上，通过 |

Windows 多的一项验证只有显式回退计划携带 MSI downgrade 属性。测试还覆盖真正的文件锁交接、SCM pending 状态不能当作已停止、缓存中断临时文件和孤儿 generation 清理、跨作用域总预算、完整签名的错误 API/schema、正确签名与包内容篡改、重定向和长度/hash 异常。所有包都是合成文本，所有暂存资源位于项目 target 内。

未执行宿主服务安装/停止/卸载、启动项修改、UAC/Polkit/Installer 授权、真实软件更新/回退或校园登录/注销。macOS 的原生编译、系统授权、安装与 Intel/Apple Silicon 运行仍需对应系统验收；当前测试覆盖其纯启动计划、错误分类、固定 bundle 包路径和跨平台 manifest 选择，不能替代原生验证。公钥/feed 的正式绑定和真实两版本升级/回退也需要发行环境验收。
