//! Thin wrapper over the `git` CLI. Auth to the private GitHub remote is delegated to the
//! user's existing git credential helper (gh / Windows Credential Manager) — we never handle tokens.
//! All functions are blocking; call from spawn_blocking.

use std::path::Path;
use std::process::Command;

fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git not found or failed to launch: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn ensure_repo(dir: &Path, remote: &str, branch: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    if !dir.join(".git").exists() {
        run_git(dir, &["init"])?;
        run_git(dir, &["checkout", "-B", branch])?;
    }
    if !remote.is_empty() {
        // set-url if origin exists, else add
        if run_git(dir, &["remote", "get-url", "origin"]).is_ok() {
            run_git(dir, &["remote", "set-url", "origin", remote])?;
        } else {
            run_git(dir, &["remote", "add", "origin", remote])?;
        }
    }
    Ok(())
}

/// Stage everything and commit. Returns Ok(false) if there was nothing to commit.
pub fn commit_all(dir: &Path, message: &str) -> Result<bool, String> {
    run_git(dir, &["add", "-A"])?;
    let status = run_git(dir, &["status", "--porcelain"])?;
    if status.trim().is_empty() {
        return Ok(false);
    }
    run_git(
        dir,
        &[
            "-c",
            "user.email=palcommand@local",
            "-c",
            "user.name=PAL COMMAND",
            "commit",
            "-m",
            message,
        ],
    )?;
    Ok(true)
}

pub fn push(dir: &Path, branch: &str) -> Result<(), String> {
    run_git(dir, &["push", "-u", "origin", branch])?;
    Ok(())
}

pub fn has_remote(dir: &Path) -> bool {
    run_git(dir, &["remote", "get-url", "origin"]).is_ok()
}
