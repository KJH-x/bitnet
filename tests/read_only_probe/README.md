# 手动授权的只读状态验证

这个独立 Cargo 工作区只供人工明确授权后的验证，不加入生产构建、自动 CI 或后台任务。它不启动 BitNet engine/runtime/GUI，不读取账号存储，不调用登录、注销、challenge、portal discovery，不更改网络或应用配置。

工具复用正式的 `bitnet_gateway::GatewayClient::status`，输入为已经单独采集并保存在私有目录中的 `bitnetReadOnly(...)` JSONP。它只替换 callback 标识为 `jsonp`，保留其余原始字节，在 `127.0.0.1` 临时监听一次请求，通过正式 HTTP 与状态解析代码回放。监听器断言精确的 `GET /cgi-bin/rad_user_info?callback=jsonp HTTP/1.1` 路径，不提供其他操作。

工具按字段比对原始 `error/user_name/online_ip/sum_bytes/sum_seconds/user_balance` 与解析后的认证状态、账号、IP、流量、时长、余额。`error` 对应解析出的 `AuthState`；数字字符串依原协议语义转换。回放与实时结果来自不同采样时刻，时长/流量可能变化，因此不把它们强制要求为同一数值。

只有显式传入 `--live-read-only` 才调用正式 `network::interfaces()` 和 `network::preferred_binding()`，核对来源网卡后，向固定默认目标发送至多一次 status GET。地址固定为 `http://10.0.0.55`，没有目标覆盖、重试或 redirect 跟随。未带该开关只回放，不枚举宿主网络，不对校园网发请求。每次状态请求限时 8 秒，整个异步验证限时 12 秒；原始输入和正式响应均最多 64 KiB。

输入与报告必须使用绝对路径，且解析后的实际路径位于本项目 `.local/live-readonly/` 内。这个目录及报告父目录必须已由执行者设置私有 ACL；工具不创建目录、不更改 ACL。报告以 `create_new` 创建，拒绝覆盖既有文件。完整账号、网卡、SSID/IP 与字段值只写入指定私有报告；终端只输出检查布尔值与固定错误标识，不显示路径、原始正文或身份信息。

```text
# 使用项目已有 target 缓存；本工具自己的锁文件最初由根 Cargo.lock 播种。
cargo build --offline --locked --manifest-path tests/read_only_probe/Cargo.toml --target-dir target

# 合成数据单元/loopback 测试，不会发送真实校园网请求。
cargo test --offline --locked --manifest-path tests/read_only_probe/Cargo.toml --target-dir target

# 仅回放：两个占位路径需替换为已设私有 ACL 目录内的绝对路径。
target/debug/bitnet-read-only-probe --jsonp <absolute-private-capture> --report <absolute-new-private-report>

# 仅在已获得用户明确授权后由人工执行，额外进行一次只读实时状态查询。
target/debug/bitnet-read-only-probe --jsonp <absolute-private-capture> --report <absolute-new-private-report> --live-read-only
```

报告成功写入并不代表所有检查通过：终端逐项列出布尔值，失败时返回非零退出码，详细结果在私有报告内。工具没有凭据或认证动作，但状态查询和私有报告仍涉及身份信息，应按用户授权范围保存，不能上传到公开 Issue、仓库或 CI 附件。
