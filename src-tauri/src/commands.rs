use crate::backup::{restore_world, run_backup, BackupReport, RestoreReport};
use crate::config_store::{
    get_secret, has_secret, load_settings, save_settings, set_secret, AppSettings,
    KEY_ADMIN_PASSWORD, KEY_FTP_PASSWORD,
};
use crate::palconfig;
use crate::rcon_ctl::{self, RconConfig, RconPlayer};
use crate::rest::RestClient;
use crate::sftp::{self, SftpConfig};
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use tauri::Manager;

fn cfg_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path().app_config_dir().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Result<AppSettings, String> {
    Ok(load_settings(&cfg_dir(&app)?))
}

#[tauri::command]
pub fn set_settings(app: tauri::AppHandle, settings: AppSettings) -> Result<(), String> {
    save_settings(&cfg_dir(&app)?, &settings)
}

#[derive(Serialize)]
pub struct SecretsPresent {
    pub ftp: bool,
    pub admin: bool,
}

#[tauri::command]
pub fn get_secrets_present() -> SecretsPresent {
    SecretsPresent {
        ftp: has_secret(KEY_FTP_PASSWORD),
        admin: has_secret(KEY_ADMIN_PASSWORD),
    }
}

#[tauri::command]
pub fn set_ftp_password(password: String) -> Result<(), String> {
    set_secret(KEY_FTP_PASSWORD, &password)
}

#[tauri::command]
pub fn set_admin_password(password: String) -> Result<(), String> {
    set_secret(KEY_ADMIN_PASSWORD, &password)
}

#[derive(Serialize)]
pub struct SftpProbe {
    pub ok: bool,
    pub message: String,
    pub sample: Vec<String>,
}

#[tauri::command]
pub async fn probe_ftp(app: tauri::AppHandle) -> Result<SftpProbe, String> {
    let s = load_settings(&cfg_dir(&app)?);
    let Some(pw) = get_secret(KEY_FTP_PASSWORD) else {
        return Ok(SftpProbe {
            ok: false,
            message: "No SFTP password saved yet.".into(),
            sample: vec![],
        });
    };
    let cfg = SftpConfig {
        host: s.sftp_host.clone(),
        port: s.sftp_port,
        user: s.sftp_user.clone(),
        password: pw,
    };
    match sftp::open(&cfg).await {
        Ok(sess) => {
            let sample = sftp::list_names(&sess, &s.save_games_path).await;
            Ok(SftpProbe {
                ok: true,
                message: format!(
                    "Connected to {}:{}. Found {} entr{} in {}.",
                    cfg.host,
                    cfg.port,
                    sample.len(),
                    if sample.len() == 1 { "y" } else { "ies" },
                    s.save_games_path
                ),
                sample,
            })
        }
        Err(e) => Ok(SftpProbe { ok: false, message: e, sample: vec![] }),
    }
}

// ---------- Live control (REST-first, RCON fallback) ----------

enum LiveChannel {
    Rest(RestClient),
    Rcon(RconConfig),
    None,
}

fn live(app: &tauri::AppHandle) -> Result<LiveChannel, String> {
    let s = load_settings(&cfg_dir(app)?);
    let pw = get_secret(KEY_ADMIN_PASSWORD)
        .ok_or("No Admin Password saved — Settings → Live Control.")?;
    if s.rest_enabled && !s.rest_url.is_empty() {
        return Ok(LiveChannel::Rest(RestClient::new(&s.rest_url, &pw)));
    }
    if s.rcon_enabled && !s.rcon_host.is_empty() {
        return Ok(LiveChannel::Rcon(RconConfig {
            host: s.rcon_host.clone(),
            port: s.rcon_port,
            password: pw,
        }));
    }
    Ok(LiveChannel::None)
}

#[derive(Serialize)]
pub struct LiveInfo {
    pub source: String,
    pub servername: Option<String>,
    pub version: Option<String>,
    pub worldguid: Option<String>,
    pub raw: Option<String>,
}

#[tauri::command]
pub async fn live_info(app: tauri::AppHandle) -> Result<LiveInfo, String> {
    match live(&app)? {
        LiveChannel::Rest(rc) => {
            let v = rc.get_json("info").await?;
            Ok(LiveInfo {
                source: "rest".into(),
                servername: v.get("servername").and_then(|x| x.as_str()).map(String::from),
                version: v.get("version").and_then(|x| x.as_str()).map(String::from),
                worldguid: v.get("worldguid").and_then(|x| x.as_str()).map(String::from),
                raw: None,
            })
        }
        LiveChannel::Rcon(rcfg) => {
            let raw = rcon_ctl::info(&rcfg).await?;
            let mut servername = None;
            let mut version = None;
            let trimmed = raw.trim();
            if let Some(cut) = trimmed.rfind('[') {
                if let Some(end) = trimmed[cut..].find(']') {
                    version = Some(trimmed[cut + 1..cut + end].to_string());
                    servername = Some(trimmed[..cut].trim().to_string());
                }
            }
            Ok(LiveInfo {
                source: "rcon".into(),
                servername: servername.or(Some(trimmed.to_string())),
                version,
                worldguid: None,
                raw: Some(raw),
            })
        }
        LiveChannel::None => Err("No live control channel enabled (REST or RCON).".into()),
    }
}

#[derive(Serialize, Default)]
pub struct LiveMetrics {
    pub source: String,
    pub currentplayernum: Option<u32>,
    pub maxplayernum: Option<u32>,
    pub serverfps: Option<f64>,
    pub serverframetime: Option<f64>,
    pub uptime: Option<u64>,
    pub basecampnum: Option<u32>,
    pub days: Option<u32>,
}

#[tauri::command]
pub async fn live_metrics(app: tauri::AppHandle) -> Result<LiveMetrics, String> {
    match live(&app)? {
        LiveChannel::Rest(rc) => {
            let v = rc.get_json("metrics").await?;
            Ok(LiveMetrics {
                source: "rest".into(),
                currentplayernum: v.get("currentplayernum").and_then(|x| x.as_u64()).map(|n| n as u32),
                maxplayernum: v.get("maxplayernum").and_then(|x| x.as_u64()).map(|n| n as u32),
                serverfps: v.get("serverfps").and_then(|x| x.as_f64()),
                serverframetime: v.get("serverframetime").and_then(|x| x.as_f64()),
                uptime: v.get("uptime").and_then(|x| x.as_u64()),
                basecampnum: v.get("basecampnum").and_then(|x| x.as_u64()).map(|n| n as u32),
                days: v.get("days").and_then(|x| x.as_u64()).map(|n| n as u32),
            })
        }
        LiveChannel::Rcon(rcfg) => {
            // RCON has no metrics endpoint — the best we can do is player count via ShowPlayers.
            let players = rcon_ctl::players(&rcfg).await.unwrap_or_default();
            Ok(LiveMetrics {
                source: "rcon".into(),
                currentplayernum: Some(players.len() as u32),
                ..Default::default()
            })
        }
        LiveChannel::None => Err("No live control channel enabled.".into()),
    }
}

#[derive(Serialize)]
pub struct LivePlayer {
    pub name: Option<String>,
    #[serde(rename = "accountName")]
    pub account_name: Option<String>,
    #[serde(rename = "playerId")]
    pub player_id: Option<String>,
    #[serde(rename = "userId")]
    pub user_id: Option<String>,
    pub ip: Option<String>,
    pub ping: Option<f64>,
    pub location_x: Option<f64>,
    pub location_y: Option<f64>,
    pub level: Option<u32>,
    pub building_count: Option<u32>,
}

#[tauri::command]
pub async fn live_players(app: tauri::AppHandle) -> Result<Vec<LivePlayer>, String> {
    match live(&app)? {
        LiveChannel::Rest(rc) => {
            let v = rc.get_json("players").await?;
            let arr = v.get("players").and_then(|x| x.as_array()).cloned().unwrap_or_default();
            Ok(arr.into_iter().map(json_to_player).collect())
        }
        LiveChannel::Rcon(rcfg) => {
            let raws = rcon_ctl::players(&rcfg).await?;
            Ok(raws.into_iter().map(rcon_to_player).collect())
        }
        LiveChannel::None => Err("No live control channel enabled.".into()),
    }
}

fn json_to_player(v: serde_json::Value) -> LivePlayer {
    let g = |k: &str| v.get(k).cloned();
    LivePlayer {
        name: g("name").and_then(|x| x.as_str().map(String::from)),
        account_name: g("accountName").and_then(|x| x.as_str().map(String::from)),
        player_id: g("playerId").and_then(|x| x.as_str().map(String::from)),
        user_id: g("userId").and_then(|x| x.as_str().map(String::from)),
        ip: g("ip").and_then(|x| x.as_str().map(String::from)),
        ping: g("ping").and_then(|x| x.as_f64()),
        location_x: g("location_x").and_then(|x| x.as_f64()),
        location_y: g("location_y").and_then(|x| x.as_f64()),
        level: g("level").and_then(|x| x.as_u64()).map(|n| n as u32),
        building_count: g("building_count").and_then(|x| x.as_u64()).map(|n| n as u32),
    }
}

fn rcon_to_player(p: RconPlayer) -> LivePlayer {
    LivePlayer {
        name: Some(p.name),
        account_name: None,
        player_id: Some(p.player_uid),
        user_id: Some(p.steam_id),
        ip: None,
        ping: None,
        location_x: None,
        location_y: None,
        level: None,
        building_count: None,
    }
}

#[tauri::command]
pub async fn live_announce(app: tauri::AppHandle, message: String) -> Result<(), String> {
    match live(&app)? {
        LiveChannel::Rest(rc) => rc.post_json("announce", &serde_json::json!({ "message": message })).await,
        LiveChannel::Rcon(rcfg) => rcon_ctl::broadcast(&rcfg, &message).await,
        LiveChannel::None => Err("No live control channel enabled.".into()),
    }
}

#[tauri::command]
pub async fn live_save(app: tauri::AppHandle) -> Result<(), String> {
    match live(&app)? {
        LiveChannel::Rest(rc) => rc.post_empty("save").await,
        LiveChannel::Rcon(rcfg) => rcon_ctl::save(&rcfg).await,
        LiveChannel::None => Err("No live control channel enabled.".into()),
    }
}

#[tauri::command]
pub async fn live_shutdown(
    app: tauri::AppHandle,
    waittime: u32,
    message: String,
) -> Result<(), String> {
    match live(&app)? {
        LiveChannel::Rest(rc) => {
            rc.post_json("shutdown", &serde_json::json!({ "waittime": waittime, "message": message })).await
        }
        LiveChannel::Rcon(rcfg) => rcon_ctl::shutdown(&rcfg, waittime, &message).await,
        LiveChannel::None => Err("No live control channel enabled.".into()),
    }
}

#[tauri::command]
pub async fn live_stop(app: tauri::AppHandle) -> Result<(), String> {
    match live(&app)? {
        LiveChannel::Rest(rc) => rc.post_empty("stop").await,
        LiveChannel::Rcon(rcfg) => rcon_ctl::do_exit(&rcfg).await,
        LiveChannel::None => Err("No live control channel enabled.".into()),
    }
}

#[tauri::command]
pub async fn live_kick(app: tauri::AppHandle, userid: String, message: String) -> Result<(), String> {
    match live(&app)? {
        LiveChannel::Rest(rc) => {
            let id = if userid.starts_with("steam_") { userid } else { format!("steam_{userid}") };
            rc.post_json("kick", &serde_json::json!({ "userid": id, "message": message })).await
        }
        LiveChannel::Rcon(rcfg) => {
            let bare = userid.trim_start_matches("steam_");
            rcon_ctl::kick(&rcfg, bare).await
        }
        LiveChannel::None => Err("No live control channel enabled.".into()),
    }
}

#[tauri::command]
pub async fn live_ban(app: tauri::AppHandle, userid: String, message: String) -> Result<(), String> {
    match live(&app)? {
        LiveChannel::Rest(rc) => {
            let id = if userid.starts_with("steam_") { userid } else { format!("steam_{userid}") };
            rc.post_json("ban", &serde_json::json!({ "userid": id, "message": message })).await
        }
        LiveChannel::Rcon(rcfg) => {
            let bare = userid.trim_start_matches("steam_");
            rcon_ctl::ban(&rcfg, bare).await
        }
        LiveChannel::None => Err("No live control channel enabled.".into()),
    }
}

// ---------- Backup ----------

#[tauri::command]
pub async fn backup_now(app: tauri::AppHandle) -> Result<BackupReport, String> {
    let s = load_settings(&cfg_dir(&app)?);
    let sftp_pw = get_secret(KEY_FTP_PASSWORD)
        .ok_or("No SFTP password saved — Settings → File Access.")?;
    let admin_pw = get_secret(KEY_ADMIN_PASSWORD);
    run_backup(s, sftp_pw, admin_pw).await
}

#[derive(Serialize)]
pub struct SafeRestartReport {
    pub backup: BackupReport,
    pub player_checks: u8,
    pub waittime: u32,
    pub recovery_seconds: u64,
}

fn require_empty_server(player_count: usize, after_backup: bool) -> Result<(), String> {
    if player_count == 0 {
        return Ok(());
    }
    if after_backup {
        Err(format!(
            "Backup completed, but restart was blocked because {player_count} player(s) joined."
        ))
    } else {
        Err(format!(
            "Restart blocked: {player_count} player(s) are online. Wait until the server is empty."
        ))
    }
}

fn require_verified_snapshot(backup: &BackupReport) -> Result<(), String> {
    if backup.worlds.is_empty() || backup.archives.is_empty() {
        Err("Restart blocked: the backup completed without a verified world snapshot.".into())
    } else if !backup.pushed {
        Err("Restart blocked: the verified backup was not pushed off-site.".into())
    } else {
        Ok(())
    }
}

#[tauri::command]
pub async fn safe_restart(app: tauri::AppHandle) -> Result<SafeRestartReport, String> {
    let before = live_players(app.clone()).await?;
    require_empty_server(before.len(), false)?;

    let mut settings = load_settings(&cfg_dir(&app)?);
    settings.stop_before_backup = false;
    let sftp_pw = get_secret(KEY_FTP_PASSWORD)
        .ok_or("No SFTP password saved - Settings > File Access.")?;
    let admin_pw = get_secret(KEY_ADMIN_PASSWORD);
    let backup = run_backup(settings, sftp_pw, admin_pw).await?;
    require_verified_snapshot(&backup)?;

    let after = live_players(app.clone()).await?;
    require_empty_server(after.len(), true)?;

    let waittime = 10;
    live_shutdown(
        app.clone(),
        waittime,
        "Verified backup complete - server restarting in 10 seconds".into(),
    )
    .await?;

    let started = std::time::Instant::now();
    let mut saw_shutdown = false;
    let recovery_seconds = loop {
        if started.elapsed().as_secs() >= 180 {
            return Err(format!(
                "Backup is safe and shutdown was accepted, but server recovery was not confirmed \
                 within 180 seconds (shutdown observed: {saw_shutdown})."
            ));
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        match live_info(app.clone()).await {
            Err(_) => saw_shutdown = true,
            Ok(_) if saw_shutdown => break started.elapsed().as_secs(),
            Ok(_) => {}
        }
    };

    Ok(SafeRestartReport {
        backup,
        player_checks: 2,
        waittime,
        recovery_seconds,
    })
}

#[cfg(test)]
mod safe_restart_tests {
    use super::{config_change_event, require_empty_server, require_verified_snapshot};
    use crate::backup::{ArchiveInfo, BackupReport};

    fn report(worlds: Vec<String>, archives: Vec<ArchiveInfo>, pushed: bool) -> BackupReport {
        BackupReport {
            timestamp: "test".into(),
            worlds,
            players: 0,
            configs: vec![],
            archives,
            warnings: vec![],
            committed: true,
            pushed,
            message: "test".into(),
        }
    }

    #[test]
    fn restart_requires_empty_server_before_and_after_backup() {
        assert!(require_empty_server(0, false).is_ok());
        assert!(require_empty_server(1, false).unwrap_err().contains("online"));
        assert!(require_empty_server(1, true).unwrap_err().contains("joined"));
    }

    #[test]
    fn restart_requires_a_verified_world_archive() {
        assert!(require_verified_snapshot(&report(vec![], vec![], true)).is_err());
        assert!(require_verified_snapshot(&report(vec!["world".into()], vec![], true)).is_err());
        assert!(require_verified_snapshot(&report(
            vec!["world".into()],
            vec![ArchiveInfo {
                name: "snapshot.tar.gz".into(),
                bytes: 1,
            }],
            false,
        ))
        .unwrap_err()
        .contains("off-site"));
        assert!(require_verified_snapshot(&report(
            vec!["world".into()],
            vec![ArchiveInfo {
                name: "snapshot.tar.gz".into(),
                bytes: 1,
            }],
            true,
        ))
        .is_ok());
    }

    #[test]
    fn config_change_events_never_include_values() {
        let updates = vec![
            ("DeathPenalty".into(), "None".into()),
            ("AdminPassword".into(), "do-not-leak-this".into()),
        ];
        let json = serde_json::to_string(&config_change_event(&updates)).unwrap();
        assert!(json.contains("AdminPassword"));
        assert!(json.contains("DeathPenalty"));
        assert!(!json.contains("do-not-leak-this"));
        assert!(!json.contains("\"None\""));
    }
}

#[tauri::command]
pub async fn restore_backup(app: tauri::AppHandle, archive_name: String) -> Result<RestoreReport, String> {
    let s = load_settings(&cfg_dir(&app)?);
    let sftp_pw = get_secret(KEY_FTP_PASSWORD)
        .ok_or("No SFTP password saved — Settings → File Access.")?;
    let admin_pw = get_secret(KEY_ADMIN_PASSWORD);
    restore_world(s, sftp_pw, admin_pw, archive_name).await
}

/// Is the backup repo pushed to an off-site remote? (Powers the "OFF-SITE / LOCAL ONLY" badge.)
#[tauri::command]
pub fn backup_offsite_status(app: tauri::AppHandle) -> bool {
    let s = load_settings(&cfg_dir(&app).unwrap_or_default());
    !s.repo_remote.is_empty()
}

#[derive(Serialize)]
pub struct BackupHistoryItem {
    pub name: String,
    pub bytes: u64,
    pub modified: String,
}

#[tauri::command]
pub fn backup_history(app: tauri::AppHandle) -> Vec<BackupHistoryItem> {
    let s = load_settings(&cfg_dir(&app).unwrap_or_default());
    if s.repo_local_path.is_empty() {
        return vec![];
    }
    let dir = PathBuf::from(&s.repo_local_path).join("saves");
    let mut items: Vec<BackupHistoryItem> = std::fs::read_dir(&dir)
        .into_iter().flatten().flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tar.gz"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let mt: chrono::DateTime<chrono::Local> = meta.modified().ok()?.into();
            Some(BackupHistoryItem {
                name: e.file_name().to_string_lossy().to_string(),
                bytes: meta.len(),
                modified: mt.format("%Y-%m-%d %H:%M:%S").to_string(),
            })
        })
        .collect();
    items.sort_by(|a, b| b.modified.cmp(&a.modified));
    items
}

// ---------- Config editor ----------

#[derive(Serialize)]
pub struct PalConfigView {
    pub pairs: Vec<(String, String)>,
    pub source: String,
}

#[derive(Serialize)]
struct ConfigChangeEvent {
    event_type: &'static str,
    source: &'static str,
    changed_keys: Vec<String>,
    created_at: String,
}

fn config_change_event(updates: &[(String, String)]) -> ConfigChangeEvent {
    let mut changed_keys: Vec<String> = updates.iter().map(|(key, _)| key.clone()).collect();
    changed_keys.sort();
    changed_keys.dedup();
    ConfigChangeEvent {
        event_type: "config_changed",
        source: "PAL COMMAND dashboard",
        changed_keys,
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn queue_config_change(app: &tauri::AppHandle, updates: &[(String, String)]) -> Result<(), String> {
    let auto_dir = cfg_dir(app)?.join("auto");
    std::fs::create_dir_all(&auto_dir).map_err(|e| e.to_string())?;
    let event = config_change_event(updates);
    let line = serde_json::to_string(&event).map_err(|e| e.to_string())?;
    let mut queue = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(auto_dir.join("discord-events.jsonl"))
        .map_err(|e| e.to_string())?;
    writeln!(queue, "{line}").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn config_load(app: tauri::AppHandle) -> Result<PalConfigView, String> {
    let s = load_settings(&cfg_dir(&app)?);
    let pw = get_secret(KEY_FTP_PASSWORD)
        .ok_or("No SFTP password saved — Settings → File Access.")?;
    let cfg = SftpConfig {
        host: s.sftp_host.clone(),
        port: s.sftp_port,
        user: s.sftp_user.clone(),
        password: pw,
    };
    let sess = sftp::open(&cfg).await?;
    let candidates = if !s.config_dir.is_empty() {
        vec![s.config_dir.clone()]
    } else {
        vec![
            "Pal/Saved/Config/LinuxServer".into(),
            "Pal/Saved/Config/WindowsServer".into(),
        ]
    };
    for cand in candidates {
        let path = format!("{cand}/PalWorldSettings.ini");
        if let Some(bytes) = sftp::download_opt(&sess, &path).await {
            let text = String::from_utf8_lossy(&bytes).to_string();
            let parsed = palconfig::parse(&text)?;
            return Ok(PalConfigView { pairs: parsed.pairs, source: path });
        }
    }
    Err("Couldn't find PalWorldSettings.ini — has the server been started at least once?".into())
}

#[tauri::command]
pub async fn config_save(
    app: tauri::AppHandle,
    updates: Vec<(String, String)>,
) -> Result<String, String> {
    if updates.is_empty() { return Ok("No changes.".into()); }
    let s = load_settings(&cfg_dir(&app)?);
    let pw = get_secret(KEY_FTP_PASSWORD)
        .ok_or("No SFTP password saved — Settings → File Access.")?;
    let cfg = SftpConfig {
        host: s.sftp_host.clone(),
        port: s.sftp_port,
        user: s.sftp_user.clone(),
        password: pw,
    };
    let sess = sftp::open(&cfg).await?;
    let candidates = if !s.config_dir.is_empty() {
        vec![s.config_dir.clone()]
    } else {
        vec![
            "Pal/Saved/Config/LinuxServer".into(),
            "Pal/Saved/Config/WindowsServer".into(),
        ]
    };
    for cand in candidates {
        let path = format!("{cand}/PalWorldSettings.ini");
        if let Some(bytes) = sftp::download_opt(&sess, &path).await {
            let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
            let backup_path = format!("{cand}/PalWorldSettings.ini.bak-{ts}");
            let _ = sftp::upload(&sess, &backup_path, &bytes).await;

            let text = String::from_utf8_lossy(&bytes).to_string();
            let mut parsed = palconfig::parse(&text)?;
            palconfig::apply_updates(&mut parsed, &updates);
            let out = palconfig::serialize(&parsed)?;
            sftp::upload(&sess, &path, out.as_bytes()).await?;
            if !s.repo_local_path.is_empty() {
                let config_dir = PathBuf::from(&s.repo_local_path).join("config");
                let _ = std::fs::create_dir_all(&config_dir);
                let (redacted, _) = crate::backup::redact_config(&out);
                let _ = std::fs::write(config_dir.join("PalWorldSettings.ini"), redacted);
            }
            let notification = queue_config_change(&app, &updates);
            let mut message = format!(
                "Saved {} change{} to {} (previous file backed up as PalWorldSettings.ini.bak-{}).",
                updates.len(),
                if updates.len() == 1 { "" } else { "s" },
                path, ts
            );
            if let Err(error) = notification {
                message.push_str(&format!(" Discord notification warning: {error}"));
            }
            return Ok(message);
        }
    }
    Err("Couldn't find PalWorldSettings.ini to update.".into())
}

// ---------- Mods ----------

#[derive(Serialize)]
pub struct ModEntry {
    pub name: String,
    pub kind: String,
    pub path: String,
}

#[tauri::command]
pub async fn mods_list(app: tauri::AppHandle) -> Result<Vec<ModEntry>, String> {
    let s = load_settings(&cfg_dir(&app)?);
    let pw = get_secret(KEY_FTP_PASSWORD)
        .ok_or("No SFTP password saved — Settings → File Access.")?;
    let cfg = SftpConfig {
        host: s.sftp_host.clone(),
        port: s.sftp_port,
        user: s.sftp_user.clone(),
        password: pw,
    };
    let sess = sftp::open(&cfg).await?;
    let mut out = Vec::new();
    let scan = [
        ("Pal/Content/Paks/~mods", "loose-pak"),
        ("Pal/Content/Paks/~WorkshopMods", "workshop"),
        ("Pal/Content/Paks/LogicMods", "logic"),
        ("Pal/Binaries/Linux/ue4ss/Mods", "ue4ss"),
        ("Pal/Binaries/Win64/ue4ss/Mods", "ue4ss"),
    ];
    for (dir, kind) in scan {
        for name in sftp::list_names(&sess, dir).await {
            out.push(ModEntry {
                name,
                kind: kind.to_string(),
                path: dir.to_string(),
            });
        }
    }
    Ok(out)
}
