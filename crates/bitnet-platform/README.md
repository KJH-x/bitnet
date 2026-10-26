# Platform integration and verification boundaries

The crate provides OS paths, credential encryption, native interface inventory,
conservative preferred IPv4 binding, network notifications and service-host integration.
It contains no service installer and never changes routes, proxies, DNS, or campus sessions.

## Credentials

- Windows uses user-scoped DPAPI with UI forbidden. System scope rejects any identity
  other than `NT SERVICE\BitNet`, rather than encrypting under an administrator's user
  account. Directories/files receive protected DACLs granting only the current identity,
  SYSTEM and Administrators. Service account profile provisioning/DPAPI across reboot
  still requires a disposable Windows service installation test.
- Linux service scope requires exactly 32 bytes at
  `$CREDENTIALS_DIRECTORY/masterkey` with safe owner/read permissions. The engine uses
  AES-256-GCM with a fresh random nonce per save and authenticated format metadata.
  systemd `LoadCredential` works on the Ubuntu 22.04 baseline; wrapping this key with
  `LoadCredentialEncrypted` is an installer concern. The installer must preserve the
  same master key across repair/upgrade. root or a complete disk/key copy is outside
  this threat model.
- Linux user scope uses `/usr/bin/secret-tool` to keep its encryption master key in
  the session Secret Service. The desktop package must depend on `libsecret-tools`
  and a functioning session Secret Service provider. Operations have a timeout and
  never fall back to a plaintext key file. A missing key over an existing encrypted
  store is an error. Headless WSL tests do not contact the desktop keyring.
- macOS user mode invokes native Security.framework generic-password APIs. System
  mode uses a socket-activated root `bitkeychain` helper and a non-login `_bitnet`
  worker. The helper checks peer UID, provisions the boot-volatile runtime directory,
  and then accesses System.keychain with interactive unlock forbidden. The worker
  authenticates the helper as UID 0. Ordinary management clients cannot call this key
  endpoint. Keychain ACLs, signed helper upgrades, launchd activation, restart and
  locked-keychain handling remain real-macOS acceptance gaps.
  Failure is reported as CredentialsUnavailable. No decrypted export API exists.

Only `Scope::User` accepts `BITNET_DATA_DIR`, which must be absolute. Tests use
project `target/` temporary directories. For Linux service simulation provide a
project-local STATE_DIRECTORY and project-local CREDENTIALS_DIRECTORY; do not put
fixture master keys or service units into real system paths.

## Network and lifecycle

- Windows interfaces/routes use IP Helper; SSID uses dynamically loaded WLAN APIs so
  an absent wireless subsystem does not become a loader dependency. SSID can be
  unavailable due to OS privacy permissions. IP interface notifications are native.
- Linux interfaces use getifaddrs, routes use kernel procfs, changes use route netlink.
  SSID reads use the read-only Wireless Extensions ioctl; unsupported drivers and
  denied access return no SSID, without a shell command or changing network state.
  UDP connect is used only for source selection and does not send a packet. A mismatch
  between selected source and the main route table is rejected, including policy-routing
  ambiguity. No arbitrary first NIC fallback is used.
- macOS enumerates interfaces with getifaddrs, reads IPv4 default gateways through
  sysctl route snapshots, and maps the kernel-selected UDP source to one unambiguous
  active interface without sending any packet. Native AF_ROUTE notifications watch
  interface/address/route changes; denied route sockets retain an open no-event
  channel so periodic polling remains available. SSID privacy access and real macOS
  runtime behavior remain acceptance gaps.
- Windows service::run hosts a callback through SCM and handles stop/shutdown with
  service::is_stopping(). The runtime must observe it and exit promptly. Unix signal
  handling belongs to the runtime. notify_ready/notify_watchdog only send notifications
  when explicitly called by the functioning runtime; no blind watchdog thread is used.
- service::status is read-only. Any installer/SCM/launchd/systemd actual service mutation
  must be tested in disposable environments, never the developer's host.

Native encrypted storage round trips and protected ACL tests can be run with
`cargo test -p bitnet-platform --lib`. Full installation, restart, identity changes,
macOS, all CPU architectures, desktop keyring prompts and long-running event handling
remain separate integration tests; compilation alone does not verify those features.
