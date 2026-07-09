//! Google OAuth2 (PKCE, "Desktop app" client type, loopback redirect).
//!
//! The user creates a Google Cloud OAuth client (Desktop app), downloads its
//! JSON, and drops it at `$XDG_CONFIG_HOME/caldav/google_client.json` — that
//! file is exactly what Google's console gives you, no hand-editing needed.
//! Tokens are cached alongside it in `tokens.json` (mode 0600).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub const SCOPES: &str = "https://www.googleapis.com/auth/calendar https://www.googleapis.com/auth/tasks";
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Debug, Deserialize)]
struct InstalledClient {
    client_id: String,
    client_secret: String,
}

#[derive(Debug, Deserialize)]
struct ClientSecretFile {
    installed: InstalledClient,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: i64,
}

fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("caldav");
    }
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".config/caldav")
}

fn client_secret_path() -> PathBuf {
    config_dir().join("google_client.json")
}

fn tokens_path() -> PathBuf {
    config_dir().join("tokens.json")
}

pub fn is_authenticated() -> bool {
    tokens_path().exists()
}

fn load_client_secret() -> Result<InstalledClient> {
    let path = client_secret_path();
    let data = std::fs::read_to_string(&path).map_err(|e| -> Box<dyn std::error::Error> {
        format!(
            "couldn't read {} ({e}). Create a Google Cloud OAuth client (Desktop app type), \
             enable the Calendar and Tasks APIs, download its JSON, and save it there.",
            path.display()
        )
        .into()
    })?;
    let file: ClientSecretFile = serde_json::from_str(&data)?;
    Ok(file.installed)
}

fn load_tokens() -> Result<TokenSet> {
    let data = std::fs::read_to_string(tokens_path())?;
    Ok(serde_json::from_str(&data)?)
}

fn save_tokens(tokens: &TokenSet) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let path = tokens_path();
    std::fs::write(&path, serde_json::to_string_pretty(tokens)?)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn random_urlsafe(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    rand::fill(bytes.as_mut_slice());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn urlenc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Blocks the calling thread until the browser redirects back with an
/// authorization code, or the user closes the tab without approving.
///
/// ponytail: no timeout on the accept() — this is a one-off, explicitly
/// user-triggered action, not something the daemon calls unattended. Add a
/// deadline if `caldavd --auth` hanging forever on an abandoned browser tab
/// becomes an actual problem.
fn receive_auth_code(listener: TcpListener, expected_state: &str) -> Result<String> {
    let (mut stream, _) = listener.accept()?;
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf)?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or("malformed callback request")?;
    let url = url::Url::parse(&format!("http://127.0.0.1{path}"))?;
    let params: std::collections::HashMap<String, String> =
        url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();

    let body = "<html><body>caldav: authorization received, you can close this tab.</body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());

    if let Some(err) = params.get("error") {
        return Err(format!("Google denied authorization: {err}").into());
    }
    if params.get("state").map(String::as_str) != Some(expected_state) {
        return Err("OAuth state mismatch (possible CSRF) — aborting".into());
    }
    params.get("code").cloned().ok_or_else(|| "no authorization code in callback".into())
}

/// Runs the full interactive flow: opens the system browser to Google's
/// consent screen, catches the redirect locally, exchanges the code for
/// tokens, and saves them. Blocking; only call this from an explicit
/// user action (`caldavd --auth`, or a TUI/GUI "connect Google" command).
pub fn authenticate() -> Result<()> {
    let client = load_client_secret()?;
    let verifier = random_urlsafe(64);
    let challenge = code_challenge(&verifier);
    let state = random_urlsafe(16);

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/");

    let auth_url = format!(
        "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}\
         &access_type=offline&prompt=consent&code_challenge={}&code_challenge_method=S256&state={}",
        urlenc(&client.client_id),
        urlenc(&redirect_uri),
        urlenc(SCOPES),
        urlenc(&challenge),
        urlenc(&state),
    );

    eprintln!("caldav: open this URL to connect your Google account:\n{auth_url}\n");
    let _ = std::process::Command::new("xdg-open").arg(&auth_url).spawn();

    let code = receive_auth_code(listener, &state)?;

    let http = reqwest::blocking::Client::new();
    let resp: TokenResponse = http
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", client.client_id.as_str()),
            ("client_secret", client.client_secret.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()?
        .error_for_status()?
        .json()?;

    let refresh_token = resp.refresh_token.ok_or(
        "Google didn't return a refresh_token. This happens if you've already authorized this \
         app before without revoking it — remove access at https://myaccount.google.com/permissions \
         and run `caldavd --auth` again.",
    )?;

    save_tokens(&TokenSet {
        access_token: resp.access_token,
        refresh_token,
        expires_at: crate::db::now() + resp.expires_in,
    })?;

    eprintln!("caldav: Google account connected.");
    Ok(())
}

fn refresh(tokens: &TokenSet) -> Result<TokenSet> {
    let client = load_client_secret()?;
    let http = reqwest::blocking::Client::new();
    let resp: TokenResponse = http
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", tokens.refresh_token.as_str()),
            ("client_id", client.client_id.as_str()),
            ("client_secret", client.client_secret.as_str()),
        ])
        .send()?
        .error_for_status()?
        .json()?;
    Ok(TokenSet {
        access_token: resp.access_token,
        refresh_token: tokens.refresh_token.clone(),
        expires_at: crate::db::now() + resp.expires_in,
    })
}

/// Returns a valid access token, transparently refreshing it if it's
/// expired (or close to it). Errors if the user hasn't run `authenticate()`
/// yet.
pub fn get_access_token() -> Result<String> {
    let tokens = load_tokens().map_err(|e| -> Box<dyn std::error::Error> {
        format!("not connected to Google yet, run `caldavd --auth` first ({e})").into()
    })?;
    if crate::db::now() >= tokens.expires_at - 60 {
        let refreshed = refresh(&tokens)?;
        save_tokens(&refreshed)?;
        return Ok(refreshed.access_token);
    }
    Ok(tokens.access_token)
}
