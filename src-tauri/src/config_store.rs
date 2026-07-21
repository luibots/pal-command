//! App settings (JSON on disk) + secrets (Windows Credential Manager via keyring).
//! Non-secret config lives in settings.json under the app config dir.
//! FTP password + Palworld AdminPassword never touch disk — they go in the OS keychain.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    // --- SFTP (file access) ---
    #[serde(alias = "ftp_host")]
    pub sftp_host: String,
    #[serde(alias = "ftp_port")]
    pub sftp_port: u16,
    #[serde(alias = "ftp_user")]
    pub sftp_user: String,
    /// Path to the SaveGames/<localuser> dir, relative to the SFTP root. World GUID folders live inside.
    pub save_games_path: String,
    /// Config dir relative to SFTP root. Empty = auto-detect LinuxServer vs WindowsServer.
    pub config_dir: String,

    // --- Palworld REST API (preferred live control if the host has opened a port) ---
    pub rest_url: String,
    pub rest_enabled: bool,

    // --- Palworld RCON (fallback live control — works on default Host Havoc port allocations) ---
    pub rcon_host: String,
    pub rcon_port: u16,
    pub rcon_enabled: bool,

    // --- GitHub backup ---
    pub repo_local_path: String,
    pub repo_remote: String,
    pub git_branch: String,
    pub backup_retention: u32,
    /// If true, gracefully stop the server (REST /shutdown or RCON DoExit) before pulling saves.
    pub stop_before_backup: bool,

    // --- Scheduler ---
    pub schedule_enabled: bool,
    pub schedule_minutes: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            sftp_host: String::new(),
            sftp_port: 22,
            sftp_user: String::new(),
            save_games_path: "Pal/Saved/SaveGames/0".into(),
            config_dir: String::new(),
            rest_url: String::new(),
            rest_enabled: false,
            rcon_host: String::new(),
            rcon_port: 25575,
            rcon_enabled: false,
            repo_local_path: String::new(),
            repo_remote: String::new(),
            git_branch: "main".into(),
            backup_retention: 20,
            stop_before_backup: false,
            schedule_enabled: false,
            schedule_minutes: 60,
        }
    }
}

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.json")
}

pub fn load_settings(config_dir: &Path) -> AppSettings {
    let p = settings_path(config_dir);
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(config_dir: &Path, s: &AppSettings) -> Result<(), String> {
    std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(settings_path(config_dir), json).map_err(|e| e.to_string())?;
    Ok(())
}

// --- Secrets: OS keychain ---

const KEYRING_SERVICE: &str = "com.luibots.palcommand";
pub const KEY_FTP_PASSWORD: &str = "ftp_password";
pub const KEY_ADMIN_PASSWORD: &str = "admin_password";

pub fn set_secret(key: &str, value: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, key).map_err(|e| e.to_string())?;
    if value.is_empty() {
        let _ = entry.delete_credential();
        Ok(())
    } else {
        entry.set_password(value).map_err(|e| e.to_string())
    }
}

pub fn get_secret(key: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, key)
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|v| !v.is_empty())
}

pub fn has_secret(key: &str) -> bool {
    get_secret(key).is_some()
}
