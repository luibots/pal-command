//! Palworld REST API client (the officially-supported control channel; RCON is deprecated).
//! Base URL like http://host:8212 ; every call is HTTP Basic auth  admin:<AdminPassword>.

use serde::Serialize;
use std::time::Duration;

pub struct RestClient {
    base: String,
    password: String,
    client: reqwest::Client,
}

impl RestClient {
    pub fn new(base: &str, password: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            password: password.to_string(),
            client: reqwest::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1/api/{}", self.base, path)
    }

    pub async fn get_json(&self, path: &str) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .get(self.url(path))
            .basic_auth("admin", Some(&self.password))
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .map_err(|e| friendly(e))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(rest_error(status.as_u16(), path));
        }
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn post_json<T: Serialize>(&self, path: &str, body: &T) -> Result<(), String> {
        let resp = self
            .client
            .post(self.url(path))
            .basic_auth("admin", Some(&self.password))
            .json(body)
            .timeout(Duration::from_secs(12))
            .send()
            .await
            .map_err(|e| friendly(e))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let mut msg = rest_error(status.as_u16(), path);
            if !body.trim().is_empty() {
                msg.push_str(&format!(" — {}", body.trim()));
            }
            return Err(msg);
        }
        Ok(())
    }

    pub async fn post_empty(&self, path: &str) -> Result<(), String> {
        self.post_json(path, &serde_json::json!({})).await
    }
}

fn rest_error(code: u16, path: &str) -> String {
    match code {
        401 => "REST auth failed (401) — check the Admin Password".to_string(),
        403 => "REST forbidden (403)".to_string(),
        404 => format!("REST endpoint /{} not found (404) — is the REST API enabled on this build?", path),
        _ => format!("REST {} on /{}", code, path),
    }
}

fn friendly(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "REST timed out — server may be down, mid-save, or the port isn't reachable".to_string()
    } else if e.is_connect() {
        "Can't reach the REST API — check the URL/port and that it's enabled + open in the firewall".to_string()
    } else {
        e.to_string()
    }
}
