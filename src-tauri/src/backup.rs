//! Backup orchestration.
//!
//! Flow: quiesce (REST /save > RCON Save > nothing) → pull over SFTP → validate the
//! Palworld save magic → compress world saves to tar.gz (configs stay raw text) →
//! retention prune → git commit/push to the private repo.

use crate::config_store::AppSettings;
use crate::rcon_ctl::{self, RconConfig};
use crate::rest::RestClient;
use crate::sftp::{self, SftpConfig};
use crate::{gitrepo, savecheck};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Serialize)]
pub struct ArchiveInfo {
    pub name: String,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct BackupReport {
    pub timestamp: String,
    pub worlds: Vec<String>,
    pub players: usize,
    pub configs: Vec<String>,
    pub archives: Vec<ArchiveInfo>,
    pub warnings: Vec<String>,
    pub committed: bool,
    pub pushed: bool,
    pub message: String,
}

struct Pulled {
    configs: Vec<(String, Vec<u8>)>,
    worlds: Vec<PulledWorld>,
}

struct PulledWorld {
    guid: String,
    level: Option<Vec<u8>>,
    others: Vec<(String, Vec<u8>)>,
}

pub async fn run_backup(
    s: AppSettings,
    sftp_pw: String,
    admin_pw: Option<String>,
) -> Result<BackupReport, String> {
    if s.sftp_host.is_empty() || s.sftp_user.is_empty() {
        return Err("Set your SFTP host and username in Settings first.".into());
    }
    if s.repo_local_path.is_empty() {
        return Err("Choose a local backup folder in Settings first.".into());
    }
    let cfg = SftpConfig {
        host: s.sftp_host.clone(),
        port: s.sftp_port,
        user: s.sftp_user.clone(),
        password: sftp_pw,
    };

    let mut warnings: Vec<String> = Vec::new();

    // --- quiesce: prefer REST /save; fall back to RCON Save; otherwise skip with a warning ---
    let have_rest = s.rest_enabled && admin_pw.is_some() && !s.rest_url.is_empty();
    let have_rcon = s.rcon_enabled && admin_pw.is_some() && !s.rcon_host.is_empty();

    if have_rest {
        let rc = RestClient::new(&s.rest_url, admin_pw.as_ref().unwrap());
        if s.stop_before_backup {
            let _ = rc.post_json("shutdown",
                &serde_json::json!({"waittime": 10, "message": "PAL COMMAND backup"})).await;
            tokio::time::sleep(Duration::from_secs(16)).await;
        } else if let Err(e) = rc.post_empty("save").await {
            warnings.push(format!("REST save failed ({e}); trying RCON."));
        } else {
            tokio::time::sleep(Duration::from_secs(6)).await;
        }
    }
    if !have_rest && have_rcon {
        let rcfg = RconConfig {
            host: s.rcon_host.clone(),
            port: s.rcon_port,
            password: admin_pw.clone().unwrap(),
        };
        if s.stop_before_backup {
            let _ = rcon_ctl::shutdown(&rcfg, 10, "PAL_COMMAND_backup").await;
            tokio::time::sleep(Duration::from_secs(16)).await;
        } else if let Err(e) = rcon_ctl::save(&rcfg).await {
            warnings.push(format!("RCON save failed ({e}); pulling anyway."));
            tokio::time::sleep(Duration::from_secs(2)).await;
        } else {
            tokio::time::sleep(Duration::from_secs(6)).await;
        }
    }
    if !have_rest && !have_rcon {
        warnings.push(
            "No live channel (REST or RCON) configured — couldn't force a save first. \
             Files may be mid-write. Enable RCON or REST for guaranteed integrity, or \
             turn on 'stop before backup'."
                .into(),
        );
    }

    // --- pull ---
    let mut pulled = pull(&cfg, &s.save_games_path, &s.config_dir).await?;

    // --- integrity check with one retry if we still have a way to resave ---
    let can_resave = !s.stop_before_backup && (have_rest || have_rcon);
    for attempt in 0..2u8 {
        let bad: Vec<String> = pulled
            .worlds
            .iter()
            .filter(|w| {
                w.level
                    .as_ref()
                    .map(|b| !savecheck::is_valid_palworld_sav(b))
                    .unwrap_or(false)
            })
            .map(|w| w.guid.clone())
            .collect();
        if bad.is_empty() {
            break;
        }
        if can_resave && attempt == 0 {
            if have_rest {
                let rc = RestClient::new(&s.rest_url, admin_pw.as_ref().unwrap());
                let _ = rc.post_empty("save").await;
            } else {
                let rcfg = RconConfig {
                    host: s.rcon_host.clone(),
                    port: s.rcon_port,
                    password: admin_pw.clone().unwrap(),
                };
                let _ = rcon_ctl::save(&rcfg).await;
            }
            tokio::time::sleep(Duration::from_secs(8)).await;
            pulled = pull(&cfg, &s.save_games_path, &s.config_dir).await?;
        } else {
            for g in &bad {
                warnings.push(format!(
                    "World {g}: Level.sav failed the integrity check (torn mid-save) — that world's \
                     snapshot was skipped this run. Turn on 'stop before backup' for a guaranteed clean copy."
                ));
            }
            for w in pulled.worlds.iter_mut() {
                if let Some(b) = &w.level {
                    if !savecheck::is_valid_palworld_sav(b) {
                        w.level = None;
                    }
                }
            }
            break;
        }
    }

    // --- write to repo ---
    let repo = PathBuf::from(&s.repo_local_path);
    let config_out = repo.join("config");
    let saves_out = repo.join("saves");
    std::fs::create_dir_all(&config_out).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&saves_out).map_err(|e| e.to_string())?;

    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();

    let mut configs_written = Vec::new();
    let mut secret_sidecar = String::new();
    for (name, data) in &pulled.configs {
        if name.ends_with(".ini") {
            let text = String::from_utf8_lossy(data);
            let (redacted, secrets) = redact_config(&text);
            std::fs::write(config_out.join(name), redacted.as_bytes()).map_err(|e| e.to_string())?;
            if !secrets.is_empty() {
                secret_sidecar.push_str(&format!("[{name}]\n"));
                for (k, v) in &secrets {
                    secret_sidecar.push_str(&format!("{k}={v}\n"));
                }
                secret_sidecar.push('\n');
            }
        } else {
            std::fs::write(config_out.join(name), data).map_err(|e| e.to_string())?;
        }
        configs_written.push(name.clone());
    }
    // Real secret values live here — NEVER committed (see .gitignore below). Restore reads it back.
    if !secret_sidecar.is_empty() {
        let header = "# PAL COMMAND — real secret values pulled from server config.\n\
                      # Git-ignored on purpose. Keep local; needed to reconstitute config on restore.\n\n";
        std::fs::write(config_out.join("secrets.local.ini"), format!("{header}{secret_sidecar}"))
            .map_err(|e| e.to_string())?;
    }

    const SIZE_WARN: usize = 95 * 1024 * 1024;
    let mut archives = Vec::new();
    let mut worlds_ok = Vec::new();
    let mut players = 0usize;
    for w in &pulled.worlds {
        let Some(level) = &w.level else { continue };
        if level.len() > SIZE_WARN {
            warnings.push(format!(
                "World {}: Level.sav is {} — approaching GitHub's 100MB hard limit. Enable Git LFS \
                 or run a save-cleanup pass.",
                &w.guid[..w.guid.len().min(8)],
                human(level.len())
            ));
        }
        let mut files = vec![("Level.sav".to_string(), level.clone())];
        for (rel, data) in &w.others {
            if rel.starts_with("Players/") {
                players += 1;
            }
            files.push((rel.clone(), data.clone()));
        }
        let archive = make_targz(&files)?;
        let short = &w.guid[..w.guid.len().min(8)];
        let fname = format!("{ts}_{short}.tar.gz");
        std::fs::write(saves_out.join(&fname), &archive).map_err(|e| e.to_string())?;
        archives.push(ArchiveInfo {
            name: fname,
            bytes: archive.len() as u64,
        });
        worlds_ok.push(w.guid.clone());
    }

    if worlds_ok.is_empty() && configs_written.is_empty() {
        return Err(
            "Nothing captured — check the SFTP paths in Settings (couldn't find Level.sav or a config file)."
                .into(),
        );
    }

    prune_saves(&saves_out, s.backup_retention as usize);
    let _ = std::fs::write(repo.join("README.md"), backup_readme());
    // Never let real secrets or the game's own rolling backup folder reach git.
    let _ = std::fs::write(
        repo.join(".gitignore"),
        "*.tmp\nsecrets.local.ini\n**/secrets.local.ini\n*.local.ini\nbackup/\n",
    );

    // --- git ---
    let message = format!(
        "backup {ts} — {} world(s), {} player(s), {} config(s)",
        worlds_ok.len(),
        players,
        configs_written.len()
    );
    let repo2 = repo.clone();
    let remote_owned = s.repo_remote.clone();
    let remote_for_task = remote_owned.clone();
    let branch = s.git_branch.clone();
    let msg2 = message.clone();
    let (committed, pushed) = tokio::task::spawn_blocking(move || -> Result<(bool, bool), String> {
        gitrepo::ensure_repo(&repo2, &remote_for_task, &branch)?;
        let committed = gitrepo::commit_all(&repo2, &msg2)?;
        let mut pushed = false;
        if committed && !remote_for_task.is_empty() && gitrepo::has_remote(&repo2) {
            gitrepo::push(&repo2, &branch)?;
            pushed = true;
        }
        Ok((committed, pushed))
    })
    .await
    .map_err(|e| e.to_string())??;

    if !remote_owned.is_empty() && !pushed && committed {
        warnings.push("Committed locally but not pushed — set a remote and check your git credentials.".into());
    }

    Ok(BackupReport {
        timestamp: ts,
        worlds: worlds_ok,
        players,
        configs: configs_written,
        archives,
        warnings,
        committed,
        pushed,
        message,
    })
}

#[derive(Debug, Serialize)]
pub struct RestoreReport {
    pub world_folder: String,
    pub files_restored: usize,
    pub server_stopped: bool,
    pub warnings: Vec<String>,
    pub message: String,
}

/// Roll the live world back to a snapshot. Stops the server first (mandatory — otherwise the
/// running server overwrites the restored files on its next autosave), then SFTP-pushes the
/// snapshot's Level.sav + LevelMeta + WorldOption + Players back into the world folder.
pub async fn restore_world(
    s: AppSettings,
    sftp_pw: String,
    admin_pw: Option<String>,
    archive_name: String,
) -> Result<RestoreReport, String> {
    let repo = PathBuf::from(&s.repo_local_path);
    let archive_path = repo.join("saves").join(&archive_name);
    if !archive_path.exists() {
        return Err(format!("Snapshot '{archive_name}' not found in the backup folder."));
    }
    let bytes = std::fs::read(&archive_path).map_err(|e| e.to_string())?;
    let files = untar_gz(&bytes)?;

    // Guard: never restore a corrupt Level.sav.
    let level = files
        .iter()
        .find(|(n, _)| n == "Level.sav")
        .ok_or("Snapshot has no Level.sav — refusing to restore.")?;
    if !savecheck::is_valid_palworld_sav(&level.1) {
        return Err("The snapshot's Level.sav fails the integrity check — refusing to restore a corrupt world.".into());
    }

    let mut warnings = Vec::new();

    // Stop the server (required). Prefer REST, then RCON.
    let have_rest = s.rest_enabled && admin_pw.is_some() && !s.rest_url.is_empty();
    let have_rcon = s.rcon_enabled && admin_pw.is_some() && !s.rcon_host.is_empty();
    let mut server_stopped = false;
    if have_rest {
        let rc = RestClient::new(&s.rest_url, admin_pw.as_ref().unwrap());
        let _ = rc
            .post_json("shutdown", &serde_json::json!({"waittime": 10, "message": "PAL COMMAND restore"}))
            .await;
        server_stopped = true;
    } else if have_rcon {
        let rcfg = RconConfig {
            host: s.rcon_host.clone(),
            port: s.rcon_port,
            password: admin_pw.clone().unwrap(),
        };
        let _ = rcon_ctl::shutdown(&rcfg, 10, "PAL_COMMAND_restore").await;
        server_stopped = true;
    } else {
        warnings.push(
            "No REST/RCON channel to stop the server — make sure it is STOPPED in the panel before \
             restoring, or the running server will overwrite the restore."
                .into(),
        );
    }
    if server_stopped {
        tokio::time::sleep(Duration::from_secs(20)).await;
    }

    // Find the world folder on the server.
    let cfg = SftpConfig {
        host: s.sftp_host.clone(),
        port: s.sftp_port,
        user: s.sftp_user.clone(),
        password: sftp_pw,
    };
    let sess = sftp::open(&cfg).await?;
    let folders = sftp::list_names(&sess, &s.save_games_path).await;
    let world = folders
        .into_iter()
        .find(|f| !f.contains('.'))
        .ok_or("Couldn't find a world folder on the server to restore into.")?;

    let mut restored = 0usize;
    for (rel, data) in &files {
        let target = format!("{}/{}/{}", s.save_games_path, world, rel);
        sftp::upload(&sess, &target, data).await?;
        restored += 1;
    }

    let message = format!(
        "Restored {} file(s) into world '{}'. {}",
        restored,
        world,
        if server_stopped {
            "Server was stopped — START it from the Host Havoc panel to load the restored world."
        } else {
            "Restore uploaded — start the server when ready."
        }
    );

    Ok(RestoreReport {
        world_folder: world,
        files_restored: restored,
        server_stopped,
        warnings,
        message,
    })
}

fn untar_gz(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let dec = GzDecoder::new(bytes);
    let mut ar = tar::Archive::new(dec);
    let mut out = Vec::new();
    for entry in ar.entries().map_err(|e| e.to_string())? {
        let mut e = entry.map_err(|e| e.to_string())?;
        let path = e
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let mut data = Vec::new();
        e.read_to_end(&mut data).map_err(|e| e.to_string())?;
        out.push((path, data));
    }
    Ok(out)
}

async fn pull(cfg: &SftpConfig, save_games_path: &str, config_dir: &str) -> Result<Pulled, String> {
    let s = sftp::open(cfg).await?;

    let candidates: Vec<String> = if !config_dir.is_empty() {
        vec![config_dir.to_string()]
    } else {
        vec![
            "Pal/Saved/Config/LinuxServer".into(),
            "Pal/Saved/Config/WindowsServer".into(),
        ]
    };
    let mut configs = Vec::new();
    for cand in &candidates {
        if let Some(main) = sftp::download_opt(&s, &format!("{cand}/PalWorldSettings.ini")).await {
            configs.push(("PalWorldSettings.ini".to_string(), main));
            for extra in ["GameUserSettings.ini", "Engine.ini"] {
                if let Some(d) = sftp::download_opt(&s, &format!("{cand}/{extra}")).await {
                    configs.push((extra.to_string(), d));
                }
            }
            break;
        }
    }

    let mut worlds = Vec::new();
    for name in sftp::list_names(&s, save_games_path).await {
        let base = format!("{save_games_path}/{name}");
        let level = sftp::download_opt(&s, &format!("{base}/Level.sav")).await;
        if level.is_none() {
            continue;
        }
        let mut others = Vec::new();
        for f in ["LevelMeta.sav", "WorldOption.sav"] {
            if let Some(d) = sftp::download_opt(&s, &format!("{base}/{f}")).await {
                others.push((f.to_string(), d));
            }
        }
        for pf in sftp::list_names(&s, &format!("{base}/Players")).await {
            if pf.ends_with(".sav") {
                if let Some(d) = sftp::download_opt(&s, &format!("{base}/Players/{pf}")).await {
                    others.push((format!("Players/{pf}"), d));
                }
            }
        }
        worlds.push(PulledWorld { guid: name, level, others });
    }
    Ok(Pulled { configs, worlds })
}

/// Config keys whose quoted values are secrets and must never reach git.
pub const SECRET_KEYS: [&str; 2] = ["AdminPassword", "ServerPassword"];

/// Replace secret values in a Palworld ini with <REDACTED>, returning the extracted originals.
pub fn redact_config(text: &str) -> (String, Vec<(String, String)>) {
    let mut out = text.to_string();
    let mut extracted = Vec::new();
    for key in SECRET_KEYS {
        let pat = format!("{key}=\"");
        if let Some(start) = out.find(&pat) {
            let vstart = start + pat.len();
            if let Some(rel_end) = out[vstart..].find('"') {
                let vend = vstart + rel_end;
                let val = out[vstart..vend].to_string();
                if val.is_empty() || val == "<REDACTED>" {
                    continue;
                }
                extracted.push((key.to_string(), val));
                out.replace_range(vstart..vend, "<REDACTED>");
            }
        }
    }
    (out, extracted)
}

/// Reverse of redact_config: put real secret values back into a redacted ini.
pub fn restore_secrets(text: &str, secrets: &[(String, String)]) -> String {
    let mut out = text.to_string();
    for (key, val) in secrets {
        let pat = format!("{key}=\"");
        if let Some(start) = out.find(&pat) {
            let vstart = start + pat.len();
            if let Some(rel_end) = out[vstart..].find('"') {
                let vend = vstart + rel_end;
                out.replace_range(vstart..vend, val);
            }
        }
    }
    out
}

fn make_targz(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut tar = tar::Builder::new(&mut enc);
        for (name, data) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            tar.append_data(&mut header, name, data.as_slice())
                .map_err(|e| e.to_string())?;
        }
        tar.finish().map_err(|e| e.to_string())?;
    }
    enc.finish().map_err(|e| e.to_string())
}

fn prune_saves(dir: &Path, keep: usize) {
    if keep == 0 { return; }
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = std::fs::read_dir(dir)
        .into_iter().flatten().flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tar.gz"))
        .filter_map(|e| { let m = e.metadata().ok()?.modified().ok()?; Some((e.path(), m)) })
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1));
    for (p, _) in files.into_iter().skip(keep) { let _ = std::fs::remove_file(p); }
}

fn human(n: usize) -> String {
    let n = n as f64;
    if n >= 1e9 { format!("{:.1} GB", n / 1e9) }
    else if n >= 1e6 { format!("{:.1} MB", n / 1e6) }
    else if n >= 1e3 { format!("{:.1} KB", n / 1e3) }
    else { format!("{n} B") }
}

fn backup_readme() -> &'static str {
    "# Palworld server backup\n\nManaged by **PAL·COMMAND**.\n\n- `config/` — server config files (PalWorldSettings.ini, GameUserSettings.ini, Engine.ini) as readable text, versioned in git.\n- `saves/` — timestamped `tar.gz` snapshots of each world (Level.sav + LevelMeta.sav + WorldOption.sav + Players/), integrity-checked before commit.\n\nSaves are binary and can be large; if a world approaches 100MB, enable Git LFS for `saves/*.tar.gz`.\n\nTo restore, extract a snapshot's files back into `Pal/Saved/SaveGames/0/<WorldGUID>/` **with the server stopped**, keeping the whole set together.\n"
}
