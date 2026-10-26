# bitnet-gateway

Transport-independent public types come from `bitnet-core`. The client exports
`new`, async `status`, `login`, and `logout` exactly as described by the project
contract. `GatewayError::into_app_error` returns static, sanitized messages.

## Transport boundaries

- HTTP/1 via Hyper and Tokio; no handwritten HTTP parser, proxy discovery, TLS,
  DNS lookup, subprocesses, or autonomous retry/authentication loop.
- Administrative endpoint must be a literal IPv4 HTTP origin. Hostnames, HTTPS,
  userinfo, path prefixes, queries, fragments, multicast and unspecified hosts
  are rejected. HTTPS is intentionally unsupported, not silently downgraded.
- Non-loopback requests require an explicit source address and interface index.
  Windows uses IP_UNICAST_IF plus GetIfEntry2 validation; Linux uses
  SO_BINDTODEVICE; macOS uses IP_BOUND_IF. All additionally bind the source IPv4.
  Any binding failure aborts without an unbound fallback.
- Linux fresh-socket unprivileged SO_BINDTODEVICE relies on kernel commit
  c427bfec18f2190b8f4718785ee8ed2db4f84ee6 (2020); old/restricted kernels fail
  closed and require a supported deployment, not automatically broader rights.
- Entire operations have one 1–60 second deadline, including discovery,
  challenge, authentication, and confirmation. Response bodies are capped at
  64 KiB and Location values at 4 KiB.
- No authentication redirect is followed. Controller discovery inspects
  same-origin Location values and can visit only fixed presentation pages.
  Controller IDs are rediscovered on every authentication/logout operation
  unless explicitly supplied by administrative configuration.

## Authentication boundaries

- Exact `jsonp(...)` callback, optional terminal semicolon and outer whitespace;
  no JavaScript evaluation, callback guessing, raw JSON or legacy CSV fallback.
- Authenticated requires `error=ok`, a nonempty username and a valid IPv4.
  Explicit `not_online_error` with no username is unauthenticated. HTTP errors,
  unknown errors, contradictory or malformed responses are errors, never
  evidence that login is needed. Statistical fields cannot imply authentication.
- Login success is confirmed using fresh status responses. A response loss can
  be reconciled with the expected account's confirmed session. Another account
  is preserved and reported as a mismatch; there is no force-login/logout API.
- No saved credentials, logging, history, GUI or service-policy dependency.
  Plaintext JSON encoding intermediates and challenge strings are zeroized.

## Verification

`cargo test -p bitnet-gateway` and
`cargo clippy -p bitnet-gateway --all-targets -- -D warnings` pass on the Windows
development host. Tests only contact literal loopback mock servers with synthetic
credentials. They cover source/interface constraints, fail-closed binding,
protocol vectors, classification, account preservation, redirects, bounded
responses, end-to-end deadlines, discovery refresh and session confirmation.

Linux/macOS implementations and target-native loopback tests are present but
have not been executed on this Windows host. Live BIT portal compatibility,
real interface changes and network-namespace isolation require separate,
explicitly authorized integration tests. No production authentication was run.

Encoding provenance and the complete upstream MIT notice are in `NOTICE`.
