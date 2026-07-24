//! App settings (JSON on disk) + secrets (Windows Credential Manager via keyring).
//! Non-secret config lives in settings.json under the app config dir. Existing
//! DPAPI-encrypted automation credentials are migrated into the keyring on first use.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

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

#[cfg(windows)]
fn read_legacy_dpapi_secret(path: &Path) -> Result<String, String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let script = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$secure = Get-Content -LiteralPath $env:PALCMD_MIGRATE_SECRET_PATH | ConvertTo-SecureString
$pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
try {
  [Console]::Out.Write([Runtime.InteropServices.Marshal]::PtrToStringBSTR($pointer))
} finally {
  [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer)
}
"#;
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("PALCMD_MIGRATE_SECRET_PATH", path)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("could not start the local DPAPI migration: {e}"))?;
    if !output.status.success() {
        return Err("the existing local DPAPI credential could not be decrypted".into());
    }
    let secret = String::from_utf8(output.stdout)
        .map_err(|_| "the existing local credential was not valid UTF-8".to_string())?;
    if secret.is_empty() {
        return Err("the existing local credential was empty".into());
    }
    Ok(secret)
}

#[cfg(not(windows))]
fn read_legacy_dpapi_secret(_path: &Path) -> Result<String, String> {
    Err("legacy DPAPI migration is only available on Windows".into())
}

pub fn migrate_legacy_secrets(config_dir: &Path) -> Result<bool, String> {
    let legacy_dir = config_dir.join("auto");
    let mappings = [
        (KEY_FTP_PASSWORD, legacy_dir.join("sftp.sec")),
        (KEY_ADMIN_PASSWORD, legacy_dir.join("admin.sec")),
    ];
    let mut migrated = false;
    for (key, path) in mappings {
        if has_secret(key) || !path.is_file() {
            continue;
        }
        let secret = read_legacy_dpapi_secret(&path)?;
        set_secret(key, &secret)?;
        migrated = true;
    }
    Ok(migrated)
}

#[cfg(all(test, windows))]
mod tests {
    use super::read_legacy_dpapi_secret;
    use std::process::Command;

    #[test]
    fn decrypts_legacy_dpapi_secret_without_plaintext_on_disk() {
        let path = std::env::temp_dir().join(format!(
            "palcommand-dpapi-test-{}.sec",
            std::process::id()
        ));
        let script = r#"
$secure = $env:PALCMD_TEST_SECRET | ConvertTo-SecureString -AsPlainText -Force
$secure | ConvertFrom-SecureString | Set-Content -LiteralPath $env:PALCMD_TEST_PATH -Encoding ascii
"#;
        let status = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("PALCMD_TEST_SECRET", "local-test-secret")
            .env("PALCMD_TEST_PATH", &path)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            read_legacy_dpapi_secret(&path).unwrap(),
            "local-test-secret"
        );
        let _ = std::fs::remove_file(path);
    }
}
