//! SFTP transport — spawns Windows 11's built-in `sftp.exe` (from `C:\Windows\System32\OpenSSH`).
//!
//! Why not a Rust crate: `russh` (and every other pure-Rust SFTP library) pulls in a
//! large crypto dep chain — zerocopy, generic-array, num-traits, curve25519-dalek — whose
//! build scripts get blocked by this machine's Windows Defender Application Control
//! policy. Shelling out to the system's own OpenSSH client avoids new build scripts
//! entirely.
//!
//! Password auth uses OpenSSH's SSH_ASKPASS mechanism: we drop a tiny `askpass.cmd`
//! helper into the app data dir that echoes an env var, then set
//!   SSH_ASKPASS = <that .cmd>
//!   SSH_ASKPASS_REQUIRE = force
//!   PALCMD_SFTP_PW = <password>
//! before spawning sftp.exe.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub struct SftpConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
}

/// A stub Session type. Unlike the earlier russh implementation there's no long-lived
/// connection — every operation spawns a fresh sftp.exe. Kept for API symmetry.
pub struct Session {
    cfg: SftpConfig,
    askpass: PathBuf,
    temp_dir: PathBuf,
}

pub async fn open(cfg: &SftpConfig) -> Result<Session, String> {
    let base = app_data_dir()?;
    let askpass = base.join("askpass.cmd");
    if !askpass.exists() {
        // Windows batch that echoes the password from an env var. OpenSSH treats
        // stdout's first line as the password. `@echo off` suppresses the command echo.
        std::fs::write(
            &askpass,
            b"@echo off\r\necho %PALCMD_SFTP_PW%\r\n",
        )
        .map_err(|e| format!("write askpass helper: {e}"))?;
    }
    let temp_dir = base.join("sftp-tmp");
    std::fs::create_dir_all(&temp_dir).map_err(|e| format!("create temp dir: {e}"))?;

    // Quick handshake check — a `ls .` batch to verify creds + host reachability.
    let sess = Session {
        cfg: SftpConfig {
            host: cfg.host.clone(),
            port: cfg.port,
            user: cfg.user.clone(),
            password: cfg.password.clone(),
        },
        askpass,
        temp_dir,
    };
    let _ = run_batch(&sess, "pwd\nbye\n").await?;
    Ok(sess)
}

pub async fn download(s: &Session, path: &str) -> Result<Vec<u8>, String> {
    let local = s.temp_dir.join(format!("dl-{}", rand_name()));
    let batch = format!(
        "get {} {}\nbye\n",
        sftp_quote(path),
        sftp_quote(local.to_string_lossy().as_ref())
    );
    let out = run_batch(s, &batch).await?;
    if !local.exists() {
        return Err(format!(
            "download {}: sftp reported success but the file wasn't created — {}",
            path,
            trim_sftp_output(&out)
        ));
    }
    let data = std::fs::read(&local).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&local);
    Ok(data)
}

pub async fn download_opt(s: &Session, path: &str) -> Option<Vec<u8>> {
    download(s, path).await.ok()
}

pub async fn upload(s: &Session, path: &str, data: &[u8]) -> Result<(), String> {
    let local = s.temp_dir.join(format!("up-{}", rand_name()));
    std::fs::write(&local, data).map_err(|e| e.to_string())?;
    let batch = format!(
        "put {} {}\nbye\n",
        sftp_quote(local.to_string_lossy().as_ref()),
        sftp_quote(path)
    );
    let result = run_batch(s, &batch).await;
    let _ = std::fs::remove_file(&local);
    result.map(|_| ())
}

pub async fn list_names(s: &Session, dir: &str) -> Vec<String> {
    // `ls -1` produces one entry per line, no formatting.
    let batch = format!("ls -1 {}\nbye\n", sftp_quote(dir));
    let out = match run_batch(s, &batch).await {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    parse_ls(&out, dir)
}

// ── internals ──────────────────────────────────────────────

async fn run_batch(s: &Session, batch: &str) -> Result<String, String> {
    let sftp_bin = find_sftp()?;
    let target = format!("{}@{}", s.cfg.user, s.cfg.host);
    let mut cmd = Command::new(sftp_bin);
    cmd.arg("-oBatchMode=no")
        .arg("-oStrictHostKeyChecking=accept-new")
        .arg("-oUserKnownHostsFile=".to_string() + &known_hosts_path()?.to_string_lossy())
        .arg("-oConnectTimeout=10")
        .arg("-oPubkeyAuthentication=no")
        .arg("-oPreferredAuthentications=password,keyboard-interactive")
        .arg("-P").arg(s.cfg.port.to_string())
        .arg("-b").arg("-")
        .arg(target)
        .env("SSH_ASKPASS", s.askpass.as_os_str())
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("DISPLAY", ":0")
        .env("PALCMD_SFTP_PW", &s.cfg.password)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Hide the console window on Windows.
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Couldn't launch sftp.exe: {e}. On Windows 11, install OpenSSH Client from Optional Features."))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(batch.as_bytes())
            .await
            .map_err(|e| format!("sftp stdin: {e}"))?;
        let _ = stdin.shutdown().await;
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("sftp wait: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        // sftp exits non-zero if any batch command fails (including a missing file).
        let msg = friendly_error(&stdout, &stderr);
        return Err(msg);
    }
    Ok(stdout + &stderr)
}

fn friendly_error(stdout: &str, stderr: &str) -> String {
    let combined = format!("{stderr}\n{stdout}");
    let low = combined.to_lowercase();
    if low.contains("permission denied") {
        return "SFTP authentication failed — check the SFTP password (same as your Host Havoc panel login).".into();
    }
    if low.contains("no such file") {
        return "SFTP: file or directory not found.".into();
    }
    if low.contains("connection refused") || low.contains("could not resolve") || low.contains("connect to") {
        return format!("SFTP: can't reach the server — {}", trim_sftp_output(&combined));
    }
    if low.contains("ssh_askpass") || low.contains("askpass") {
        return "SFTP: password helper didn't run. Make sure Windows OpenSSH Client is installed (Settings → Apps → Optional Features).".into();
    }
    trim_sftp_output(&combined)
}

fn trim_sftp_output(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("sftp>"))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn parse_ls(out: &str, requested_dir: &str) -> Vec<String> {
    let mut names = Vec::new();
    let dir_prefix = requested_dir.trim_end_matches('/');
    for raw in out.lines() {
        let line = raw.trim();
        // Skip prompts, blank lines, the "Fetching …" progress, and echoed commands.
        if line.is_empty()
            || line.starts_with("sftp>")
            || line.starts_with("Fetching")
            || line.starts_with("Sending")
            || line.starts_with("Connected")
            || line.starts_with("Changing")
            || line.starts_with("Remote working directory")
            || line.starts_with("Warning:")
        {
            continue;
        }
        // `ls -1` output — either "name" or "path/name" depending on how sftp prints it.
        let name = line
            .trim_start_matches(dir_prefix)
            .trim_start_matches('/')
            .to_string();
        if name.is_empty() || name == "." || name == ".." || name.contains(' ') && name.contains(':') {
            // Skip stat-formatted lines that leaked in (should not happen with -1).
            continue;
        }
        names.push(name);
    }
    names
}

fn sftp_quote(p: &str) -> String {
    if p.contains(' ') || p.contains('\\') {
        format!("\"{}\"", p.replace('"', "\\\""))
    } else {
        p.to_string()
    }
}

fn rand_name() -> String {
    // Cheap unique name — combines nanosecond time with process id.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{}", nanos, std::process::id())
}

fn find_sftp() -> Result<PathBuf, String> {
    // Windows 11 ships OpenSSH client at this path when installed.
    let default = Path::new(r"C:\Windows\System32\OpenSSH\sftp.exe");
    if default.exists() {
        return Ok(default.to_path_buf());
    }
    // Fall back to PATH lookup.
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("sftp.exe");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    Err(
        "sftp.exe not found. Install OpenSSH Client via Settings → Apps → Optional Features → \
         'OpenSSH Client', then reopen PAL·COMMAND."
            .into(),
    )
}

fn app_data_dir() -> Result<PathBuf, String> {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("APPDATA"))
        .map_err(|_| "no LOCALAPPDATA env var".to_string())?;
    let dir = PathBuf::from(base).join("com.luibots.palcommand");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create app data dir: {e}"))?;
    Ok(dir)
}

fn known_hosts_path() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join("known_hosts"))
}
