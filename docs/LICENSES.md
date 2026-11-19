# 分发许可文件

项目本体采用根目录 `LICENSE` 中的 MIT 文本，版权行为 `Copyright (c) 2026 BitNet contributors`。网关协议适配的来源、上游作者版权和 MIT 文本另外保留在 `crates/bitnet-gateway/NOTICE`。两份原文均随客户端、系统后台及便携包分发。

`scripts/generate-notices.py` 使用 Python 标准库和 Cargo 的离线依赖数据，读取当前机器已有的依赖缓存。默认选择 `rustc` 报告的宿主目标，发布时应显式指定目标。缺少目标依赖缓存时返回错误。

```text
python scripts/generate-notices.py --target x86_64-pc-windows-msvc --strict
python scripts/generate-notices.py --target x86_64-pc-windows-msvc --output-dir build/licenses/windows-x64 --strict
```

默认生成 `build/licenses/THIRD-PARTY-NOTICES.txt` 和 `build/licenses/THIRD-PARTY-NOTICES.json`。不同目标并行打包必须分别指定输出目录，防止相互覆盖。TXT 保存项目许可、依赖的名称/版本/来源/SPDX 表达式、原文索引和完整原始许可/声明文本；相同原始字节的文本按 SHA256 只收录一次。JSON 同时保存原文、原始文件字节哈希、编码、依赖特性、来源和收集问题。不会选择或改写双许可条件，也不会用标准模板替换缺失的作者版权。

收录选定目标上全部工作区成员的普通依赖和构建依赖。`cargo tree` 确定实际启用的包，`cargo metadata` 提供版本、来源和许可信息。交叉编译的构建依赖也可能来自宿主目标。多个来源使用同一包名与版本时，收录解析结果中所有匹配项。系统库、另外静态链接的库、字体及安装器自身带入的组件单独核对。

缺少原文、无 SPDX 信息、不可读文件或扫描上限均写入报告。默认仍生成报告供检查；`--strict` 在写完报告后以状态码 2 阻止含收集问题的发布。运行/输入错误以状态码 1 返回。文件及目录扫描有边界，拒绝包目录外的许可符号链接、空文件、二进制 NUL 和超限文档。JSON 不包含个人缓存绝对路径。

## 没随 crate 打包的上游许可

对于同仓库子 crate，只有仓库 URL、完整提交和 SPDX 表达式一致，才关联根 crate 随包携带的许可。注册表依赖从 `.cargo_vcs_info.json` 读取提交，Git 依赖用本地 `git` 核对缓存 checkout 的 HEAD 和 Cargo 锁定的提交。报告记录来源 crate、提交及原文哈希。

若原 crate 包未带原文，人工读取官方仓库对应完整提交的许可文本，可保留在 `docs/licenses/`，在 `upstream-provenance.json` 登记依赖名称/版本、仓库、提交、准确原文 URL、文件名和 SHA256。生成器始终离线，只在这些字段与当前 Cargo 依赖逐项一致且本地文件哈希相符时使用；不匹配便保留缺项。不得用默认分支或仅同名协议冒充该版本的许可。

## 桌面字体

GUI 内嵌 Adobe Source Han Sans SC 2.001 的 Regular 与 Medium 子集，原始文件来自上游 `2.001R`，提交为 `a69eda09a2fb57836e055bddb0a9a61b162ad1e6`。字体采用 SIL OFL 1.1，并保留原作者版权。子集的主名称改为 `BitNet Han Sans SC` 和 `BitNet Han Sans SC Medium`，遵守上游对 `Source` 的保留名称要求。

两个子集合计 3,913,660 字节，覆盖 ASCII、Latin-1、标点、箭头、全角符号与 GB2312 一级汉字。源文件和产物的 URL、SHA-256、生成工具版本保存在 [字体清单](../crates/bitnet-gui/assets/fonts/manifest.json)，完整原始 [OFL 文本](../crates/bitnet-gui/assets/fonts/LICENSE.txt) 随桌面包分发为 `FONT-OFL.txt`。许可生成器核对字体字节和许可证哈希，并将原文写入第三方声明。

复现命令和运行时家族名见 [字体资产说明](../crates/bitnet-gui/assets/fonts/README.md)。

## 额外静态库

Linux 发布构建静态链接 OpenSSL，必须除 Rust 包装 crate 之外另带实际库的版权文件和其引用的完整 Apache-2.0 文本。例如在项目中保留已提取的原始文件后：

```text
python scripts/generate-notices.py --target x86_64-unknown-linux-gnu --output-dir build/licenses/linux-amd64 --extra-license OpenSSL=.local/linux-openssl/OpenSSL-COPYRIGHT.txt --extra-license OpenSSL-Apache-2.0=.local/linux-openssl/OpenSSL-APACHE-2.0.txt --strict
```

`--extra-license [LABEL=]PATH` 可重复使用，额外文本也按原字节计算哈希；未提供的版本/SPDX 不会猜测。打包时应依据实际链接库及版本检查这些来源。OpenSSL 的开发包版权文件可能是指向运行库文档的相对符号链接，应读取其真实目标，不能把 Windows 上不可读的零字节链接当作版权原文。

工作区使用的 `libsqlite3-sys` 自带 SQLite。其包装 crate 的 MIT 文本不能代替 SQLite 自身的声明，因此生成器另外保留本地包 `sqlite3/sqlite3.h` 的实际首段注释，包含作者的版权放弃声明。报告记录整个源文件 SHA256、原始字节范围和行范围；不改写、不从网上推断声明。包中另带的其他许可文件可能属于未启用的供应商代码，保守收录不代表其实际被链接。

## 分发位置与验证

- Windows 的 Client/Service MSI 各自安装目录的 `licenses/`。
- Windows/Linux 便携包的 `licenses/`。
- Linux 的 `/usr/share/doc/bitnet-core/` 与 `/usr/share/doc/bitnet-gui/`，按各包独立归属。
- macOS App 的 `Contents/Resources/licenses/`，系统后台的 `/Library/PrivilegedHelperTools/BitNetLicenses/`。

至少包含项目 `LICENSE`、网关 `NOTICE`、`THIRD-PARTY-NOTICES.txt`；JSON 可作为发布审计附件保留。打包只复制已生成文件，不在用户安装时访问网络或运行许可收集工具。离线提取测试为 `python scripts/test-generate-notices.py`；实际包还需检查文件清单。
