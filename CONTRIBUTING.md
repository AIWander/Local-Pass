# Contributing to Local-Pass

Thanks for thinking about contributing — this project is alpha and issues + PRs are very welcome. Because Local-Pass exposes a machine to the internet, **security-sensitive changes get extra scrutiny**.

## Quick start

```bash
git clone https://github.com/AIWander/Local-Pass
cd Local-Pass
cargo build --release
```

Requires Rust 1.75+.

## Ground rules

- Open an issue before a big change so we can discuss approach
- Keep PRs focused — one concern per PR
- Match existing code style (`cargo fmt` + `cargo clippy` clean)
- Add a test if you fix a bug or add behavior
- Update docs (README, doc comments) when behavior changes
- **Security changes need a design note in the PR description** — what's the threat model, what's the new attack surface, how is it mitigated

## Running checks locally

```bash
cargo fmt --check
cargo clippy --release -- -D warnings
cargo test --release
```

CI runs the same on x64 and ARM64.

## Reporting bugs

Use the [bug report template](.github/ISSUE_TEMPLATE/bug_report.yml). Include:

- Windows version + architecture (x64 / ARM64)
- `local-pass.exe --version` output
- Tunnel provider (ngrok / Tailscale / Cloudflare / direct)
- Steps to reproduce, expected vs actual behavior
- Relevant logs from `./.local-pass/` (but redact your bearer token first)

## Reporting security issues

**Critical:** see [SECURITY.md](SECURITY.md). **Do not open a public issue** for security reports. Local-Pass exposes a remote attack surface and we take security reports seriously.

## Code of Conduct

By participating, you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

Contributions are licensed under [MIT](LICENSE).
