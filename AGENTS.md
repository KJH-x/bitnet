# BitNet implementation

The accepted design is in docs/ARCHITECTURE.md. Keep daemon, GUI, CLI and maintenance dependencies isolated. GUI and CLI use the same Command/Reply model; never add independent authentication loops to either frontend.

- Never run real campus login/logout or alter host networking during development. Use loopback fixtures and explicit simulation.
- Never log passwords, challenge tokens, protocol info, full authentication URLs, decrypted credential stores, or request bodies. Secret Debug must remain redacted.
- Authentication status, network environment and internet health are independent. A failed probe is never permission to log out.
- Service endpoints, trust roots and executable paths are administrative configuration. Public management cannot return saved passwords.
- System service installation/startup is tested only in disposable environments; do not register production services on the user's host for routine tests.
- Do not put files outside this project. Build products, test state and downloaded tools belong under ignored target/, build/, dist/ or .local/.
- Do not mark untested platform support, installers, signing, live campus behavior, or soak tests as verified. Record evidence and gaps in docs/STATUS.md.
