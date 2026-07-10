use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use crate::models::AppError;

// ── OAuth Configuration ──

pub const DEFAULT_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const DEFAULT_AUTHORIZE_URL: &str = "https://platform.claude.com/oauth/authorize";
pub const DEFAULT_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const DEFAULT_CALLBACK_PORT: u16 = 4545;
pub const DEFAULT_SCOPES: &[&str] = &["user:profile", "user:inference"];

// ── PKCE ──

pub struct PkceChallenge {
    pub verifier: String,
    pub challenge: String,
}

pub fn generate_pkce_pair() -> PkceChallenge {
    let verifier = generate_random_token(32);
    let challenge = {
        let hash = Sha256::digest(verifier.as_bytes());
        URL_SAFE_NO_PAD.encode(hash)
    };
    PkceChallenge {
        verifier,
        challenge,
    }
}

pub fn generate_random_token(len: usize) -> String {
    let bytes: Vec<u8> = (0..len).map(|_| rand::rng().random::<u8>()).collect();
    URL_SAFE_NO_PAD.encode(&bytes)
}

// ── Authorization URL ──

pub fn build_authorize_url(
    pkce: &PkceChallenge,
    state: &str,
    callback_port: u16,
) -> String {
    let redirect_uri = format!("http://localhost:{callback_port}/callback");
    let scopes = DEFAULT_SCOPES.join(" ");

    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        DEFAULT_AUTHORIZE_URL,
        percent_encode(DEFAULT_CLIENT_ID),
        percent_encode(&redirect_uri),
        percent_encode(&scopes),
        percent_encode(state),
        percent_encode(&pkce.challenge),
    )
}

// ── Callback Server ──

pub struct CallbackResult {
    pub code: String,
    pub state: String,
}

/// Starts a one-shot TCP listener on localhost:{port}/callback.
/// Blocks until a request arrives or timeout is reached.
pub fn wait_for_callback(port: u16, timeout_secs: u64) -> Result<CallbackResult, AppError> {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))
        .map_err(|e| AppError::Internal(format!("Failed to bind callback port {port}: {e}")))?;

    listener
        .set_nonblocking(false)
        .map_err(|e| AppError::Internal(format!("Failed to set blocking: {e}")))?;

    let timeout = std::time::Duration::from_secs(timeout_secs);
    let start = std::time::Instant::now();

    // Poll for connections with small sleep intervals to allow timeout
    listener
        .set_nonblocking(true)
        .map_err(|e| AppError::Internal(format!("Failed to set non-blocking: {e}")))?;

    loop {
        if start.elapsed() > timeout {
            return Err(AppError::Internal(
                "OAuth callback timed out — no response from browser".to_string(),
            ));
        }

        match listener.accept() {
            Ok((stream, _)) => {
                // Set the stream to blocking for reading
                stream
                    .set_nonblocking(false)
                    .map_err(|e| AppError::Internal(format!("Stream error: {e}")))?;
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .ok();

                return handle_callback_connection(stream);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            Err(e) => {
                return Err(AppError::Internal(format!("Listener error: {e}")));
            }
        }
    }
}

fn handle_callback_connection(mut stream: std::net::TcpStream) -> Result<CallbackResult, AppError> {
    let reader = BufReader::new(&stream);
    let request_line = reader
        .lines()
        .next()
        .ok_or_else(|| AppError::Internal("Empty request".to_string()))?
        .map_err(|e| AppError::Internal(format!("Read error: {e}")))?;

    // Send success response immediately
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n\
        <html><body style=\"font-family:system-ui;text-align:center;padding:40px\">\
        <h2>Login successful!</h2>\
        <p>You can close this window and return to AgentOffice.</p>\
        </body></html>";
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();

    // Parse: GET /callback?code=XXX&state=YYY HTTP/1.1
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(AppError::Internal("Malformed HTTP request".to_string()));
    }

    let path = parts[1];
    let query = path
        .split('?')
        .nth(1)
        .ok_or_else(|| AppError::Internal("No query parameters in callback".to_string()))?;

    let mut code = None;
    let mut state = None;

    for param in query.split('&') {
        let mut kv = param.splitn(2, '=');
        match (kv.next(), kv.next()) {
            (Some("code"), Some(v)) => code = Some(v.to_string()),
            (Some("state"), Some(v)) => state = Some(v.to_string()),
            _ => {}
        }
    }

    Ok(CallbackResult {
        code: code.ok_or_else(|| AppError::Internal("Missing 'code' in callback".to_string()))?,
        state: state
            .ok_or_else(|| AppError::Internal("Missing 'state' in callback".to_string()))?,
    })
}

// ── Token Exchange ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenSet {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

pub async fn exchange_code(
    code: &str,
    verifier: &str,
    callback_port: u16,
) -> Result<OAuthTokenSet, AppError> {
    let redirect_uri = format!("http://localhost:{callback_port}/callback");

    let client = reqwest::Client::new();
    let response = client
        .post(DEFAULT_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(
            form_encode(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &redirect_uri),
                ("client_id", DEFAULT_CLIENT_ID),
                ("code_verifier", verifier),
            ]),
        )
        .send()
        .await
        .map_err(|e| AppError::Provider(format!("Token exchange request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Provider(format!(
            "Token exchange failed ({status}): {body}"
        )));
    }

    let token_resp: TokenResponse = response
        .json()
        .await
        .map_err(|e| AppError::Provider(format!("Failed to parse token response: {e}")))?;

    let expires_at = token_resp.expires_in.map(|secs| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + secs
    });

    Ok(OAuthTokenSet {
        access_token: token_resp.access_token,
        refresh_token: token_resp.refresh_token,
        expires_at,
        scopes: Some(DEFAULT_SCOPES.iter().map(|s| s.to_string()).collect()),
    })
}

pub async fn refresh_token(token_set: &OAuthTokenSet) -> Result<OAuthTokenSet, AppError> {
    let refresh = token_set
        .refresh_token
        .as_deref()
        .ok_or_else(|| AppError::Provider("No refresh token available".to_string()))?;

    let scopes = DEFAULT_SCOPES.join(" ");

    let client = reqwest::Client::new();
    let response = client
        .post(DEFAULT_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(
            form_encode(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh),
                ("client_id", DEFAULT_CLIENT_ID),
                ("scope", &scopes),
            ]),
        )
        .send()
        .await
        .map_err(|e| AppError::Provider(format!("Token refresh request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Provider(format!(
            "Token refresh failed ({status}): {body}"
        )));
    }

    let token_resp: TokenResponse = response
        .json()
        .await
        .map_err(|e| AppError::Provider(format!("Failed to parse refresh response: {e}")))?;

    let expires_at = token_resp.expires_in.map(|secs| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + secs
    });

    Ok(OAuthTokenSet {
        access_token: token_resp.access_token,
        // Preserve old refresh_token if new one not provided
        refresh_token: token_resp
            .refresh_token
            .or_else(|| token_set.refresh_token.clone()),
        expires_at,
        scopes: token_set.scopes.clone(),
    })
}

/// Check if a token set is expired.
pub fn is_token_expired(token_set: &OAuthTokenSet) -> bool {
    match token_set.expires_at {
        Some(expires_at) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            // Consider expired 60 seconds early to avoid edge cases
            now >= expires_at.saturating_sub(60)
        }
        None => false, // No expiry means assume valid
    }
}

// ── Helpers ──

fn percent_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    result
}

fn form_encode(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}
