# Local-Pass

> Expose your Windows machine to a remote AI (Claude / ChatGPT / mobile) over a secure tunnel. **Your computer, but reachable from your phone.**

**Status:** alpha. For users who want to keep working on their home machine from anywhere — through Claude.ai, ChatGPT custom actions, or any MCP-aware client.

[![Build](https://github.com/AIWander/Local-Pass/actions/workflows/build.yml/badge.svg)](https://github.com/AIWander/Local-Pass/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Platform: Windows](https://img.shields.io/badge/Platform-Windows%20x64%20%7C%20ARM64-blue.svg)](https://github.com/AIWander/Local-Pass/releases)

## The flow

```
You, on your phone or laptop, logged into Claude.ai or ChatGPT
                    ↓ HTTPS + bearer token
        Tunnel provider (ngrok / Tailscale / Cloudflare)
                    ↓ tunneled to 127.0.0.1:9100
        local-pass.exe (HTTP/SSE MCP server) on your home machine
                    ↓
        Your tools: files, shell, screen capture, http, registry, shortcuts
```

Your AI on the road feels like your AI at home.

## What it does

`local-pass` is a single-binary MCP server that exposes your Windows machine to a remote AI via tunneled URL.

| Category | Tools |
|---|---|
| **Files** | read_file, write_file, append_file, copy_file, move_file, create_dir, list_dir, search_file, tail_file |
| **Shells** | powershell, bash, run, smart_exec, chain |
| **Persistent shells** | psession_create, psession_destroy, psession_history, psession_list, psession_read, psession_run |
| **HTTP** | http_request, http_download, http_fetch, http_scrape |
| **Screen viewer** *(opt-in)* | screen_capture, screen_list_monitors, screen_text_ocr |
| **Recovery** | session_checkpoint, session_recover, recovery_status |
| **Personal Windows** | shortcut_run, registry_read, clipboard_read, clipboard_write |
| **Transforms** | grep, find_replace, extract_lines, json_format, csv↔json, base64_*, hash_file |
| **Git (safe-mode)** | branch, checkout, commit, diff, log, status, stash |
| **Utility** | port_check, kill_process, list_process, notify, sqlite_query, system_info, archive_*, md2docx |

~60 tools total. Single static-linked .exe. No external dependencies.

## Install

### Option 1 — Portable (recommended)

1. Download `local-pass-windows-x64.zip` (or `arm64`) from [Releases](https://github.com/AIWander/Local-Pass/releases/latest)
2. Extract to `C:\tools\local-pass\`
3. Generate an auth token + start the server:
   ```powershell
   C:\tools\local-pass\local-pass.exe serve --bind 127.0.0.1:9100
   ```
   First run prints your **bearer token** (saved to `./.local-pass/auth.token`, chmod 600). Save it — you'll need it in the AI client.
4. Start a tunnel (pick one below)
5. Add the tunnel URL + bearer token as an MCP server in your AI client

### Option 2 — MSI installer

1. Download `local-pass-windows-x64.msi` from [Releases](https://github.com/AIWander/Local-Pass/releases/latest)
2. Run it. The MSI installs to `C:\Program Files\Local-Pass\` and registers Start Menu shortcuts.
3. Run `Local-Pass: Start Server` from the Start Menu (it'll print your bearer token + tunnel suggestions)

### Option 3 — Have your AI install it for you

Open Claude / ChatGPT / your local LLM and paste:

> Install **AIWander/Local-Pass** on my Windows machine using the AI install runbook at <https://github.com/AIWander/Local-Pass#for-ai-assistants>

## Tunneling

`local-pass` doesn't ship a tunnel — pick whichever provider fits your use case.

### ngrok (easiest, free tier, random URL)

```powershell
# In one terminal:
C:\tools\local-pass\local-pass.exe serve --bind 127.0.0.1:9100

# In another:
ngrok http 9100
# Copy the https://abc123.ngrok-free.app URL → use as your MCP server URL in Claude/ChatGPT
```

Free tier: random URL on each restart. Paid: reserved domain.

### Tailscale (most secure, requires tailnet)

```powershell
C:\tools\local-pass\local-pass.exe serve --bind 127.0.0.1:9100
tailscale serve --bg https / http://localhost:9100
# Reachable on your tailnet at https://<machine-name>.<tailnet>.ts.net
```

Only devices on your tailnet can reach it. End-to-end encrypted via WireGuard. The bearer token is still required — defense in depth.

### Cloudflare Tunnel (stable URL, requires CF account)

```powershell
C:\tools\local-pass\local-pass.exe serve --bind 127.0.0.1:9100

# Quick tunnel (random URL):
cloudflared tunnel --url http://localhost:9100

# Named tunnel (stable URL on your domain):
cloudflared tunnel create local-pass
cloudflared tunnel route dns local-pass home.yourdomain.com
cloudflared tunnel run local-pass
```

## Connect from Claude / ChatGPT

### Claude.ai

Settings → Integrations → Add MCP Server:
- URL: `https://<your-tunnel-url>/mcp`
- Auth header: `Authorization: Bearer <your-token>`

### ChatGPT (Custom Action)

GPT Builder → Configure → Add Actions → Import schema from URL:
- Schema URL: `https://<your-tunnel-url>/.well-known/mcp-schema.json`
- Authentication: API Key → Custom header `Authorization` with value `Bearer <your-token>`

## Screen viewer

**Off by default.** Enable with:

```powershell
local-pass.exe serve --bind 127.0.0.1:9100 --enable-screen-preview
```

When enabled:

- **`screen_capture`** — explicit tool the AI calls to grab a PNG of the current screen (scaled 0.5x by default for bandwidth)
- **`screen_text_ocr`** — Windows OCR over a screen region, returns text
- **`screen_list_monitors`** — multi-monitor info
- **Auto-after-output** — every tool's response includes a small `screen_after` PNG so the AI sees "where it left off" visually after each command. This is what makes remote operation feel continuous.

**Safety rails:**
- Rate limited to 12 captures/minute (configurable)
- Every capture appended to `./.local-pass/screen_audit.log` with timestamp + originating tool
- Auto-after-output captures are scaled to 0.25x by default (smaller than explicit captures)
- Once you trust the AI's behavior, you can disable auto-after-output via `--no-auto-screen` while keeping explicit `screen_capture` available — or disable screen entirely by restarting without `--enable-screen-preview`

## Auth model

- Bearer token generated on first run, stored at `./.local-pass/auth.token` (chmod 600 on POSIX-equivalent)
- Every HTTP request requires `Authorization: Bearer <token>` header
- Token rotation: `local-pass.exe rotate-token` regenerates and invalidates the old one
- Audit log of every authenticated call at `./.local-pass/access.log`
- Failed-auth attempts logged at `./.local-pass/auth_failures.log`; auto-bans IP after 10 failures in 60s

The tunnel provider gives you TLS; the bearer token gives you identity. **Both matter** — never expose `local-pass` directly on a public IP without auth.

## For AI Assistants

If you're an AI installing this for a user, run these steps in order:

### 1. Detect environment

```powershell
$arch = if ((Get-CimInstance Win32_Processor).Architecture -eq 12) { "arm64" } else { "x64" }
Write-Host "Architecture: $arch"
```

### 2. Download + extract

```powershell
$url = "https://github.com/AIWander/Local-Pass/releases/latest/download/local-pass-windows-$arch.zip"
Invoke-WebRequest $url -OutFile $env:TEMP\local-pass.zip
New-Item -ItemType Directory -Force -Path C:\tools\local-pass | Out-Null
Expand-Archive $env:TEMP\local-pass.zip -DestinationPath C:\tools\local-pass -Force
```

### 3. Generate auth token (first run prints it)

```powershell
C:\tools\local-pass\local-pass.exe init
# Token printed to console + saved to ./.local-pass/auth.token
```

### 4. Help the user pick a tunnel

Ask which the user prefers:
- **ngrok** if they want easiest setup (free, random URL)
- **Tailscale** if they already have a tailnet (most secure)
- **Cloudflare Tunnel** if they own a domain (stable URL)

### 5. Start the server

```powershell
Start-Process -FilePath C:\tools\local-pass\local-pass.exe -ArgumentList "serve --bind 127.0.0.1:9100" -WindowStyle Hidden
```

### 6. Verify

```powershell
Invoke-WebRequest http://127.0.0.1:9100/health -Headers @{Authorization = "Bearer $(Get-Content C:\tools\local-pass\.local-pass\auth.token)"}
# Expect: 200 OK with {"status":"ok"}
```

### 7. Tell the user

"Local-Pass is running on your machine on port 9100. Your bearer token is in `C:\tools\local-pass\.local-pass\auth.token`. Start a tunnel (ngrok / tailscale / cloudflared) pointing to localhost:9100, then add the public URL + bearer token to your remote AI as an MCP server."

## Uninstall

```powershell
# Stop the server if running
Get-Process local-pass -ErrorAction SilentlyContinue | Stop-Process

# Remove install
Remove-Item C:\tools\local-pass -Recurse -Force

# Remove any tunnel config you set up (see your tunnel provider's docs)
```

## State directory

`local-pass` keeps its state in `./.local-pass/`:

- `auth.token` — bearer token (chmod 600 equivalent)
- `access.log` — every authenticated request
- `auth_failures.log` — failed auth attempts
- `screen_audit.log` — every screen capture, with timestamp + originating tool
- `breadcrumbs/` — multi-step operation tracking (CPC-compatible)
- `psession/` — persistent PowerShell session state
- `recovery/` — checkpoint state for crash recovery

Fully portable: copy the folder to another machine and your state goes with it.

## Build from source

```bash
git clone https://github.com/AIWander/Local-Pass
cd Local-Pass
cargo build --release
# Binary at: target/release/local-pass.exe
```

Requires Rust 1.75+.

## Companion repos

- [**AIWander/Programmer-Wander**](https://github.com/AIWander/Programmer-Wander) — single-AI dev shell (stdio MCP, local-only)
- [**AIWander/Universal-Ops**](https://github.com/AIWander/Universal-Ops) — multi-AI orchestrator (manager + ops + dashboard) for delegated coding

Local-Pass is the **remote access** companion. Use it when you're away from your machine; use Universal-Ops when you're at it and want to delegate; use Programmer-Wander when you want a dev shell without the orchestration layer.

All three are independent — install any combination.

## Security notes

Local-Pass exposes your machine to the internet (via your tunnel). Defaults are conservative:

- Bearer token required on every request
- Auto-ban after repeated auth failures
- Screen viewer off by default
- Rate limits on screen capture
- Audit log of every call
- Recommend: bind to `127.0.0.1` and rely on the tunnel for public exposure, never `0.0.0.0` directly

Review [SECURITY.md](SECURITY.md) before publishing your tunnel URL.

## Manual operation (until v1 ships)

Local-Pass v0.1.0-alpha is **scaffold-only** — `local-pass.exe serve` doesn't yet start a real HTTP/SSE server, the screen viewer is unimplemented, and no auth code is wired up. While we build the real implementation, here's how to achieve the same end-result manually with off-the-shelf tools:

1. **Run an HTTP MCP server** of your choice on a local port (e.g., `http://localhost:9100`). Several open-source MCP servers ship HTTP/SSE transports today.
2. **Add bearer-token auth** at a reverse-proxy layer (Caddy or nginx with a simple auth header check) since most MCP servers don't ship bearer auth out of the box.
3. **Tunnel it** with ngrok / Tailscale / Cloudflare Tunnel as documented in the [Tunneling](#tunneling) section above.
4. **Add the URL + bearer token** to your AI client (Claude.ai integration / ChatGPT custom action) as an MCP server.

When v1 ships, `local-pass.exe serve` replaces steps 1–2 in a single command: the binary does HTTP/SSE MCP, owns bearer-token auth + IP-ban + audit log, exposes your tools, and (optionally) the screen viewer. Steps 3–4 are intentionally still up to you — tunnel choice is yours.

Implementation iterates here in the open. PRs and design discussion welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) and the [Auth model](#auth-model) + [Screen viewer](#screen-viewer) sections above for the design.

## License

MIT. See [LICENSE](LICENSE).
