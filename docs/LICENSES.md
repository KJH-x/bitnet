# 分发许可文件

项目本体采用根目录 `LICENSE` 中的 MIT 文本，版权行为 `Copyright (c) 2026 BitNet contributors`。网关协议适配的来源、上游作者版权和 MIT 文本另外保留在 `crates/bitnet-gateway/NOTICE`。两份原文均随客户端、系统后台及便携包分发。

`scripts/generate-notices.py` 使用 Python 标准库和 `cargo metadata --offline --locked`，只读当前机器已有的依赖缓存，不安装工具、不修改依赖、不联网。默认选择 `rustc` 报告的宿主目标，发布时应显式指定目标。缺少目标依赖缓存时失败，不自动下载。

```text
python scripts/generate-notices.py --target x86_64-pc-windows-msvc --strict
python scripts/generate-notices.py --target x86_64-pc-windows-msvc --output-dir build/licenses/windows-x64 --strict
```

默认生成 `build/licenses/THIRD-PARTY-NOTICES.txt` 和 `build/licenses/THIRD-PARTY-NOTICES.json`。不同目标并行打包必须分别指定输出目录，防止相互覆盖。TXT 保存项目许可、依赖的名称/版本/来源/SPDX 表达式、原文索引和完整原始许可/声明文本；相同原始字节的文本按 SHA256 只收录一次。JSON 同时保存原文、原始文件字节哈希、编码、依赖特性、来源和收集问题。不会选择或改写双许可条件，也不会用标准模板替换缺失的作者版权。

范围为选定目标上全部工作区成员的普通依赖和构建依赖闭包，排除仅开发依赖的边。构建依赖保守收录；Cargo 的特性可能包含解析时的开发特性统一结果。这份资料不是精确到最终链接符号的 SBOM，也不证明发布包已完成所有许可审计。系统库、另外静态链接的库、字体及安装器自身带入的组件需要另外核对。

缺少原文、无 SPDX 信息、不可读文件或扫描上限均写入报告。默认仍生成报告供检查；`--strict` 在写完报告后以状态码 2 阻止含收集问题的发布。运行/输入错误以状态码 1 返回。文件及目录扫描有边界，拒绝包目录外的许可符号链接、空文件、二进制 NUL 和超限文档。JSON 不包含个人缓存绝对路径。

## 没随 crate 打包的上游许可

对于同仓库子 crate，只有仓库 URL、`.cargo_vcs_info.json` 的完整提交和 SPDX 表达式完全一致，才关联到本地缓存中该仓库根 crate 所携带的根许可。报告记录来源 crate、提交及原文哈希。不同提交不会沿用这个关联。

若原 crate 包未带原文，人工读取官方仓库对应完整提交的许可文本，可保留在 `docs/licenses/`，在 `upstream-provenance.json` 登记依赖名称/版本、仓库、提交、准确原文 URL、文件名和 SHA256。生成器始终离线，只在这些字段与当前 Cargo 依赖逐项一致且本地文件哈希相符时使用；不匹配便保留缺项。不得用默认分支或仅同名协议冒充该版本的许可。

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
