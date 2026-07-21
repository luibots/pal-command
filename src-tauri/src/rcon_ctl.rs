//! Minimal Source RCON client for Palworld — pure tokio, no crates beyond what the
//! project already uses. Zero new build scripts (WDAC-friendly).
//!
//! Packet format (little-endian):
//!   [size:i32] [id:i32] [type:i32] [body:cstring] [pad:u8=0]
//!   size = 4 (id) + 4 (type) + len(body) + 1 (body nul) + 1 (pad)
//!
//! Types:  3 = SERVERDATA_AUTH,  2 = SERVERDATA_EXECCOMMAND,
//!         2 = SERVERDATA_RESPONSE_VALUE (also 0 in some servers),
//!         AUTH_RESPONSE from the server has type 2 and id = original id on success,
//!         id = -1 on auth failure.
//!
//! Palworld quirks handled here:
//!  - Broadcast/Shutdown message parser splits on spaces server-side. `escape_broadcast`
//!    substitutes underscores as the well-known workaround.
//!  - We time out reads at 5s so a mid-save server doesn't hang the UI.

use serde::Serialize;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

const TYPE_AUTH: i32 = 3;
const TYPE_EXEC: i32 = 2;

pub struct RconConfig {
    pub host: String,
    pub port: u16,
    pub password: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct RconPlayer {
    pub name: String,
    pub player_uid: String,
    pub steam_id: String,
}

async fn send_packet(stream: &mut TcpStream, id: i32, ptype: i32, body: &str) -> Result<(), String> {
    let body_bytes = body.as_bytes();
    let size = 4 + 4 + body_bytes.len() as i32 + 2;
    let mut buf = Vec::with_capacity(size as usize + 4);
    buf.extend_from_slice(&size.to_le_bytes());
    buf.extend_from_slice(&id.to_le_bytes());
    buf.extend_from_slice(&ptype.to_le_bytes());
    buf.extend_from_slice(body_bytes);
    buf.push(0);
    buf.push(0);
    stream.write_all(&buf).await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn read_packet(stream: &mut TcpStream) -> Result<(i32, i32, String), String> {
    let mut size_buf = [0u8; 4];
    stream.read_exact(&mut size_buf).await.map_err(|e| e.to_string())?;
    let size = i32::from_le_bytes(size_buf);
    if !(10..=8192).contains(&size) {
        return Err(format!("RCON: bogus packet size {}", size));
    }
    let mut rest = vec![0u8; size as usize];
    stream.read_exact(&mut rest).await.map_err(|e| e.to_string())?;
    if rest.len() < 10 {
        return Err("RCON: packet too small".into());
    }
    let id = i32::from_le_bytes(rest[0..4].try_into().unwrap());
    let ptype = i32::from_le_bytes(rest[4..8].try_into().unwrap());
    let body_end = rest[8..].iter().position(|&b| b == 0).unwrap_or(rest.len() - 8);
    let body = String::from_utf8_lossy(&rest[8..8 + body_end]).to_string();
    Ok((id, ptype, body))
}

pub async fn cmd(cfg: &RconConfig, command: &str) -> Result<String, String> {
    let addr = format!("{}:{}", cfg.host, cfg.port);
    let mut stream = timeout(Duration::from_secs(6), TcpStream::connect(&addr))
        .await
        .map_err(|_| format!("RCON connect to {} timed out", addr))?
        .map_err(|e| format!("RCON connect to {} failed: {}", addr, e))?;

    // AUTH
    send_packet(&mut stream, 1, TYPE_AUTH, &cfg.password).await?;
    // Some servers send an empty RESPONSE_VALUE before the AUTH_RESPONSE. Read up to 2.
    let mut authed = false;
    for _ in 0..2 {
        let (id, _ptype, _body) = timeout(Duration::from_secs(5), read_packet(&mut stream))
            .await
            .map_err(|_| "RCON auth timed out".to_string())??;
        if id == -1 {
            return Err("RCON auth failed — wrong password (uses PalWorldSettings.ini AdminPassword)".into());
        }
        if id == 1 {
            authed = true;
            break;
        }
    }
    if !authed {
        return Err("RCON auth: server didn't confirm auth".into());
    }

    // EXEC
    send_packet(&mut stream, 2, TYPE_EXEC, command).await?;
    let (_id, _ptype, body) = timeout(Duration::from_secs(6), read_packet(&mut stream))
        .await
        .map_err(|_| format!("RCON '{}' response timed out", command))??;
    Ok(body)
}

pub async fn info(cfg: &RconConfig) -> Result<String, String> {
    cmd(cfg, "Info").await
}

pub async fn players(cfg: &RconConfig) -> Result<Vec<RconPlayer>, String> {
    let raw = cmd(cfg, "ShowPlayers").await?;
    Ok(parse_players(&raw))
}

fn parse_players(raw: &str) -> Vec<RconPlayer> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !l.to_lowercase().starts_with("name,"))
        .filter_map(|l| {
            let parts: Vec<&str> = l.splitn(3, ',').collect();
            if parts.len() != 3 {
                return None;
            }
            Some(RconPlayer {
                name: parts[0].trim().to_string(),
                player_uid: parts[1].trim().to_string(),
                steam_id: parts[2].trim().to_string(),
            })
        })
        .collect()
}

pub fn escape_broadcast(msg: &str) -> String {
    msg.replace(' ', "_")
}

pub async fn broadcast(cfg: &RconConfig, msg: &str) -> Result<(), String> {
    cmd(cfg, &format!("Broadcast {}", escape_broadcast(msg))).await?;
    Ok(())
}

pub async fn save(cfg: &RconConfig) -> Result<(), String> {
    cmd(cfg, "Save").await?;
    Ok(())
}

pub async fn shutdown(cfg: &RconConfig, seconds: u32, msg: &str) -> Result<(), String> {
    cmd(cfg, &format!("Shutdown {} {}", seconds, escape_broadcast(msg))).await?;
    Ok(())
}

pub async fn do_exit(cfg: &RconConfig) -> Result<(), String> {
    cmd(cfg, "DoExit").await?;
    Ok(())
}

pub async fn kick(cfg: &RconConfig, steam_id: &str) -> Result<(), String> {
    cmd(cfg, &format!("KickPlayer {}", steam_id)).await?;
    Ok(())
}

pub async fn ban(cfg: &RconConfig, steam_id: &str) -> Result<(), String> {
    cmd(cfg, &format!("BanPlayer {}", steam_id)).await?;
    Ok(())
}
