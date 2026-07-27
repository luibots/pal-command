# PAL·COMMAND

**An agentic command & control center for game servers — starting with Palworld.**

Not a config panel. A platform: manage a live server, keep it backed up off-site,
build and organize your own mods, and (soon) hand the boring admin work to an AI copilot
that can actually *see* your server.

> Status: v0, in active development. Desktop app (Tauri + React). Windows-first.

---

## Why it's a desktop app (on purpose)

A hosted web dashboard physically can't do the things that make this useful — run the
mod toolchain (`repak`, `UAssetGUI`) against your local Steam install, open a raw SFTP
socket to your host, or drop a `.pak` into a game folder. PAL·COMMAND runs on the admin's
machine so it can reach all three: **your files, your game, your server.**

## The pillars

| Pillar | What it does | Status |
|---|---|---|
| **Dashboard** | Live telemetry plus guarded restart: require zero players, verify and push a backup, recheck players, restart, and confirm recovery | ✅ Built |
| **Live control** | REST API (preferred) + hand-rolled Source RCON fallback — announce, kick/ban, save, shutdown | ✅ Built |
| **Backups** | Force-save → SFTP pull → integrity-check (`PlZ` magic) → compress → retention → git commit/push. Secret-redacted, restore built in. | ✅ Built |
| **Config editor** | Round-trip-safe `PalWorldSettings.ini` editor — preserves unknown keys, validates before write, pre-write backup | ✅ Built |
| **Mod Studio** | GUI for the DataTable → repack → deploy pipeline. Extract a table, edit values, one-click build + backup + push. | 🔜 Next |
| **AI copilot** | Chat that reads live state + config and *drafts* changes for your approval. DeepSeek + local Ollama, provider-pluggable. | 🔜 Planned |
| **Multi-server + installer** | Manage servers as profiles; one-click mod installer to share with your guild | 🔜 Planned |

Config saves made in PAL COMMAND notify the Discord alert channel with changed setting
names only. Values are never included. Scheduled backups also compare the redacted
configuration and announce changes made outside the dashboard. `/status` includes the
most recently delivered config-change summary.

On Windows, PAL COMMAND automatically imports existing DPAPI-encrypted credentials
from its `auto` directory into Windows Credential Manager. Existing backup and bot
installations therefore do not require passwords to be entered again.

The supervised Discord bot detects player joins through the Palworld REST API. It
asks the loopback Pal Companion service to construct a contextual welcome from the
server name, world day, online roster, and persistent visit history, then broadcasts
the result in game. If the constructor is unavailable, the bot falls back to a short
local template instead of dropping the welcome.

### Open PAL COMMAND with Palworld

Install the per-user auto-launch task once:

```powershell
.\scripts\Install-PalworldAutoLaunch.ps1
```

A hidden lightweight watcher starts at Windows sign-in. It opens PAL COMMAND once when
Palworld starts, never launches a duplicate, and does not reopen the dashboard if you
close it during the same game session. Remove the behavior with:

```powershell
.\scripts\Install-PalworldAutoLaunch.ps1 -Remove
```

## Architecture

```
Tauri desktop app
├── src/  (React 19 + TS + Vite)
│   ├── App.tsx            — shell, tabs, live telemetry poll
│   └── views/             — Dashboard · Backup · Config · Settings
└── src-tauri/  (Rust 2021, Tokio, Tauri v2)
    ├── sftp.rs            — SFTP via the system OpenSSH client (no heavy crypto deps)
    ├── rcon_ctl.rs        — Source RCON, hand-rolled in pure tokio
    ├── rest.rs            — Palworld REST API client
    ├── palconfig.rs       — round-trip parser for the one-line OptionSettings format
    ├── savecheck.rs       — Palworld save-file integrity check
    ├── backup.rs          — backup + restore orchestration, secret redaction
    ├── gitrepo.rs         — git CLI wrapper (auth via the user's own credential helper)
    ├── config_store.rs    — settings (JSON) + secrets (OS keychain)
    └── commands.rs        — the Tauri command surface
```

## Security posture

- **Secrets never hit disk in plaintext.** SFTP password + server AdminPassword live in the
  Windows Credential Manager (`keyring`).
- **Backups are secret-safe.** `AdminPassword` / `ServerPassword` are redacted from any
  committed config; real values stay in a git-ignored sidecar used only for restore.
- **Git push** delegates auth to your existing credential helper — no tokens in the app.
- No server credentials, IPs, or personal data are committed to this repo.

## Design language

Dark tactical console — near-black ground, amber signal accent, Impact display type.
Semantic color for state (green healthy / red critical). It should feel like a mission
control panel, not a settings page.

---

*This repo is the platform. World-save data lives in a separate private repo.*
