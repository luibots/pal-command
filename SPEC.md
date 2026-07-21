# PAL·COMMAND — Product Spec

> Master control tower for a Host Havoc Palworld dedicated server.
> Status: v0 in progress. Last updated 2026-07-14.

## The pitch

You rent a Palworld dedicated server from Host Havoc. Their panel gives you FTP + a
"Restart" button and nothing else. PAL·COMMAND is a Tauri desktop app that gives you a
proper mission control:

- **Backups** — safe, integrity-checked snapshots pushed to your private GitHub repo on a schedule
- **Live control** — force-save, broadcast, kick, ban, graceful shutdown (via the Palworld REST API)
- **Config editor** — round-trip-safe editor for `PalWorldSettings.ini` with a curated view + full raw table
- **Mods dashboard** — list what's actually installed across every mod location

## Non-goals (v0)

- **Starting** the server. Host Havoc has no customer API; only the panel can boot it.
  We can *stop* the server (via REST); starting stays a panel click.
- Real-time chat / player-position streaming. REST is polled at 8–12s intervals.
- Config *live-reload*. Palworld reads settings at boot only; the app enforces the
  stop-edit-start cycle in the UI.

## Architecture

```
Tauri desktop app
├── src/  (React 19 + TS + Vite 7)
│   ├── App.tsx        — top bar / tabs / status bar / first-run wizard
│   └── views/
│       ├── Dashboard  — REST info, metrics, players, actions, activity log
│       ├── Backup     — run now, history, next scheduled run, integrity mode
│       ├── Config     — featured settings + full raw editor
│       └── Settings   — connections, secrets, backup + schedule
└── src-tauri/  (Rust 2021, Tokio, Tauri v2)
    ├── ftp.rs         — suppaftp blocking client (Host Havoc = plain FTP on high port)
    ├── rest.rs        — Palworld REST client (Basic auth, tight error messages)
    ├── palconfig.rs   — round-trip parser for the one-line OptionSettings=(...) format
    ├── savecheck.rs   — Palworld save magic (PlZ) integrity check
    ├── backup.rs      — quiesce → pull → validate → compress → retention → git
    ├── gitrepo.rs     — thin `git` CLI wrapper (delegates auth to git credential helper)
    ├── config_store.rs — JSON settings + keyring secrets
    └── commands.rs    — every Tauri #[command]
```

## Verified facts driving the design

1. **Palworld save paths.** Saves at `Pal/Saved/SaveGames/0/<WorldGUID>/` — enumerate the
   GUID subfolder, don't hardcode. Capture `Level.sav` + `LevelMeta.sav` +
   `WorldOption.sav` + the whole `Players/` folder as a *set* (character↔world linkage).
   Exclude the game's own `backup/` subfolder.
2. **The tearing bug.** Copying `Level.sav` while the ~30s autosave is running produces
   the classic corruption ("too many null bytes"). PAL·COMMAND forces a REST `/save` and
   waits, then verifies the `PlZ` magic (offset 8) before trusting the file. If it fails,
   it retries once. Optional "stop before backup" mode for guaranteed integrity.
3. **Level.sav can exceed 100MB.** Monolithic zlib blob; changes wholesale per save.
   → World saves stored as timestamped `tar.gz` snapshots with a rotating retention
   window. Configs (`.ini`) commit raw so diffs are readable. Size guard warns at 95MB.
4. **PalWorldSettings.ini is fragile.** Two lines; one giant `OptionSettings=(...)` on
   line 2. A single malformed token silently reverts *every* setting to defaults.
   → Round-trip parser preserves unknown keys verbatim; serializer validates balanced
   parens + even quote count before writing; pre-write `.bak-YYYYMMDD-HHMMSS` backup on
   the server. Server must be **stopped** to edit.
5. **REST > RCON, always.** Pocketpair officially deprecated RCON — it's scheduled to
   stop functioning in a future update, and it has a broadcast-truncates-at-first-space
   bug + non-ASCII name mangling. REST is polled from `:8212/v1/api/*`, Basic auth
   `admin:<AdminPassword>`. `userid` fields need the `steam_` prefix.
6. **Host Havoc has no customer API.** TCAdmin only exposes an internal billing API to
   the host, not to customers. FTP credentials = panel login (rotate that panel password
   after every leak, and never expose FTP to the internet).

## Security

- All secrets (FTP password, `AdminPassword`) go into the Windows Credential Manager via
  the `keyring` crate. They never touch `settings.json` and never appear in logs.
- Git push delegates auth to the user's existing credential helper (gh / Windows Credential
  Manager) — no personal access tokens in-app.
- `.gitignore` in the backup repo. Ships without hardcoded personal info.

## Roadmap

- **v0.2**: Restore flow (browse snapshots, extract, upload with server stopped)
- **v0.3**: Save-size cleanup (integrate magicbear/palworld-server-toolkit's orphaned-container prune)
- **v0.4**: Optional Host Havoc panel automation for the *start* button (browser-driven, opt-in)
