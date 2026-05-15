# Security Policy

Local-Pass exposes your Windows machine to a remote AI over a tunnel. **Security is the project's first concern.** Please read this whole file before publishing your tunnel URL.

## Reporting a vulnerability

If you find a security issue in Local-Pass, email **josephwander@gmail.com** directly. **Do not** open a public GitHub issue.

We aim to:
- Acknowledge within 24 hours
- Triage within 3 days
- Ship a fix or mitigation for high-severity issues within 7 days
- Coordinate disclosure with the reporter

## Threat model

Local-Pass assumes:

- The user has **explicit consent** of the machine owner to expose it remotely
- The tunnel provider (ngrok / Tailscale / Cloudflare) provides TLS in transit
- The bearer token is treated as a high-value secret (akin to an SSH private key)
- The user reviews and trusts the AI client they connect (Claude, ChatGPT, etc.)

What Local-Pass defends against by default:

- Anonymous access (bearer token required on every request)
- Brute-force auth (auto-ban IP after 10 failures in 60s)
- Excessive screen capture (rate-limited; off by default)
- Forgotten cleanup (every action is audit-logged)

What Local-Pass does NOT defend against:

- A compromised tunnel provider
- A compromised AI client (e.g., a malicious "MCP server" config in Claude that intercepts requests)
- A user willingly sharing their bearer token
- OS-level attacks on the host machine
- Bugs in third-party crates (report those upstream)

## In scope

- The `local-pass.exe` binary and all its subcommands
- The HTTP/SSE MCP server implementation
- The auth token generation, storage, and validation
- The screen viewer subsystem and its rate limiting
- The auto-ban logic for failed authentications
- The `./.local-pass/` state directory permissions
- Shipping artifacts on the Releases page

## Out of scope

- Third-party tunnel providers (ngrok, Tailscale, Cloudflare)
- Third-party MCP clients (Claude.ai, ChatGPT, etc.)
- The user's host operating system
- Issues in Rust, Cargo, or third-party crates (report those upstream)

## Hardening recommendations for users

Beyond Local-Pass's defaults, you should:

1. **Bind to localhost only** (`--bind 127.0.0.1:9100`, never `0.0.0.0`). The tunnel handles public exposure.
2. **Rotate your bearer token periodically** with `local-pass.exe rotate-token`.
3. **Run `local-pass` as a non-admin user** — never run as Administrator unless you need a specific privileged action.
4. **Use Tailscale or named Cloudflare Tunnels** rather than random ngrok URLs when you can — they're harder to discover and easier to revoke.
5. **Keep screen viewer off** unless you actively need visual feedback. Turn it back off when you're done.
6. **Review the audit log periodically** (`./.local-pass/access.log` and `screen_audit.log`).
7. **Stop the server when you're not using it.** `local-pass` doesn't auto-start by design — start it only when you'll be remote.

## Disclosure

After a fix lands, we'll credit the reporter (with permission) in release notes and the GitHub Security Advisory.
