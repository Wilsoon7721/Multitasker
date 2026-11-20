use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::remote::RemoteError;

/// How long to wait for the person to finish signing in in their browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// A sign-in service and how this app is registered with it.
#[derive(Debug, Clone)]
pub struct OAuthApp {
    pub authorize_url: &'static str,
    pub token_url: &'static str,
    pub scopes: &'static str,
    /// Extra authorize parameters, e.g. Google's `access_type=offline`.
    pub extra: &'static [(&'static str, &'static str)],
    /// The loopback host the service expects in redirect URIs (Google: 127.0.0.1, Microsoft: localhost).
    pub redirect_host: &'static str,
    pub client_id: String,
    /// Google's desktop clients have one; it isn't confidential in installed apps, but the token endpoint wants it.
    pub client_secret: Option<String>,
}

impl OAuthApp {
    pub fn google(client_id: String, client_secret: Option<String>) -> Self {
        OAuthApp {
            authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
            token_url: "https://oauth2.googleapis.com/token",
            scopes: "openid email https://www.googleapis.com/auth/drive.appdata",
            extra: &[("access_type", "offline"), ("prompt", "consent")],
            redirect_host: "127.0.0.1",
            client_id,
            client_secret,
        }
    }

    /// Personal Microsoft accounts only: OneDrive's app folder permission doesn't exist for work accounts.
    pub fn microsoft(client_id: String) -> Self {
        OAuthApp {
            authorize_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize",
            token_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
            scopes: "openid email profile offline_access Files.ReadWrite.AppFolder",
            extra: &[("prompt", "select_account")],
            redirect_host: "localhost",
            client_id,
            client_secret: None,
        }
    }
}

/// What a finished sign-in yields.
#[derive(Debug, Clone)]
pub struct SignedIn {
    pub refresh_token: String,
    pub access_token: String,
    pub expires_in: u64,
    /// The account's email (or name), from the sign-in's ID token, for showing who's signed in.
    pub account: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    id_token: Option<String>,
}

/// Signs in through the browser: opens the service's page, waits for its redirect to a local port, trades the code.
pub fn sign_in(app: &OAuthApp, open_browser: impl FnOnce(&str) -> Result<(), String>, cancel: &AtomicBool) -> Result<SignedIn, RemoteError> {
    let listener = TcpListener::bind((if app.redirect_host == "localhost" { "127.0.0.1" } else { app.redirect_host }, 0))?;
    let redirect_uri = format!("http://{}:{}/", app.redirect_host, listener.local_addr()?.port());
    let verifier = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let state = uuid::Uuid::new_v4().simple().to_string();

    let mut params = vec![
        ("client_id", app.client_id.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("response_type", "code"),
        ("scope", app.scopes),
        ("state", state.as_str()),
        ("code_challenge_method", "S256"),
    ];
    let challenge = pkce_challenge(&verifier);
    params.push(("code_challenge", challenge.as_str()));
    params.extend(app.extra.iter().copied());
    let url = format!("{}?{}", app.authorize_url, form_encode(&params));

    open_browser(&url).map_err(RemoteError::Io)?;
    let code = wait_for_code(&listener, &state, cancel)?;

    let mut form = vec![
        ("client_id", app.client_id.as_str()),
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_verifier", verifier.as_str()),
    ];
    if let Some(secret) = app.client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    let token = post_token(app, &form)?;
    let refresh_token = token
        .refresh_token
        .ok_or_else(|| RemoteError::Io("The service didn't allow staying signed in; try signing in again.".into()))?;
    Ok(SignedIn {
        account: token.id_token.as_deref().and_then(account_from_id_token),
        access_token: token.access_token,
        expires_in: token.expires_in.unwrap_or(3600),
        refresh_token,
    })
}

fn post_token(app: &OAuthApp, form: &[(&str, &str)]) -> Result<TokenResponse, RemoteError> {
    let agent: ureq::Agent =
        ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
    let mut response = agent.post(app.token_url).send_form(form.iter().copied()).map_err(|e| RemoteError::Io(e.to_string()))?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().map_err(|e| RemoteError::Io(e.to_string()))?;
    if status != 200 {
        return Err(RemoteError::Auth(oauth_error(&body).unwrap_or_else(|| format!("Sign-in failed ({status})."))));
    }
    serde_json::from_str(&body).map_err(|e| RemoteError::Io(format!("Unexpected sign-in response: {e}")))
}

fn oauth_error(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let code = v.get("error")?.as_str()?;
    let detail = v.get("error_description").and_then(|d| d.as_str()).unwrap_or(code);
    Some(match code {
        "invalid_grant" => "The sign-in has expired or was revoked; sign in again.".to_owned(),
        _ => detail.lines().next().unwrap_or(code).to_owned(),
    })
}

/// Waits for the browser to come back with `?code=…&state=…`, answering it with a page that says it can be closed.
fn wait_for_code(listener: &TcpListener, state: &str, cancel: &AtomicBool) -> Result<String, RemoteError> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(RemoteError::Io("Sign-in was cancelled.".into()));
        }
        if Instant::now() > deadline {
            return Err(RemoteError::Io("Sign-in timed out; try again.".into()));
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(result) = answer(stream, state)? {
                    return result;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.into()),
        }
    }
}

/// Handles one browser request; `None` for requests that aren't the redirect (a favicon, say).
fn answer(mut stream: TcpStream, state: &str) -> Result<Option<Result<String, RemoteError>>, RemoteError> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let target = line.split_whitespace().nth(1).unwrap_or("/");
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let params: Vec<(String, String)> = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (form_decode(k), form_decode(v)))
        .collect();
    let get = |key: &str| params.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());

    let outcome = if let Some(error) = get("error") {
        Some(Err(RemoteError::Io(if error == "access_denied" {
            "Sign-in was cancelled.".into()
        } else {
            format!("Sign-in failed: {}", get("error_description").unwrap_or(error))
        })))
    } else if let Some(code) = get("code") {
        if get("state").as_deref() == Some(state) {
            Some(Ok(code))
        } else {
            Some(Err(RemoteError::Io("Sign-in came back from an unexpected page; try again.".into())))
        }
    } else {
        None
    };

    let (status, message) = match &outcome {
        Some(Ok(_)) => ("200 OK", "You're signed in to MultiTasker. You can close this tab."),
        Some(Err(_)) => ("200 OK", "Sign-in didn't finish. You can close this tab and try again in MultiTasker."),
        None => ("404 Not Found", "Not found."),
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>MultiTasker</title>\
         <body style=\"font-family:system-ui;display:grid;place-items:center;height:90vh\"><p>{message}</p>"
    );
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len());
    Ok(outcome)
}

/// Keeps a fresh access token from a stored refresh token.
pub struct TokenSource {
    app: OAuthApp,
    refresh_token: Mutex<String>,
    cached: Mutex<Option<(String, Instant)>>,
    /// Called when the service hands out a new refresh token (Microsoft rotates them), so it can be saved.
    on_rotate: Box<dyn Fn(&str) + Send + Sync>,
}

impl TokenSource {
    pub fn new(app: OAuthApp, refresh_token: String, on_rotate: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self { app, refresh_token: Mutex::new(refresh_token), cached: Mutex::new(None), on_rotate: Box::new(on_rotate) }
    }

    /// Starts with the access token a sign-in just returned, saving a refresh round trip.
    pub fn with_access_token(self, token: String, expires_in: u64) -> Self {
        *self.cached.lock().unwrap_or_else(|e| e.into_inner()) = Some((token, Instant::now() + Duration::from_secs(expires_in)));
        self
    }

    pub fn access_token(&self) -> Result<String, RemoteError> {
        let mut cached = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        // Renew a minute early so a token never expires mid-request.
        if let Some((token, expires)) = cached.as_ref() {
            if Instant::now() + Duration::from_secs(60) < *expires {
                return Ok(token.clone());
            }
        }
        let refresh = self.refresh_token.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut form = vec![
            ("client_id", self.app.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
        ];
        if let Some(secret) = self.app.client_secret.as_deref() {
            form.push(("client_secret", secret));
        }
        if self.app.client_secret.is_none() {
            form.push(("scope", self.app.scopes));
        }
        let response = post_token(&self.app, &form)?;
        if let Some(rotated) = response.refresh_token.filter(|r| r != &refresh) {
            (self.on_rotate)(&rotated);
            *self.refresh_token.lock().unwrap_or_else(|e| e.into_inner()) = rotated;
        }
        let expires = Instant::now() + Duration::from_secs(response.expires_in.unwrap_or(3600));
        *cached = Some((response.access_token.clone(), expires));
        Ok(response.access_token)
    }

    /// Forgets the cached access token, e.g. after the service rejected it.
    pub fn invalidate(&self) {
        *self.cached.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// The email (or name) in an ID token; it's only a display label, so the signature isn't checked.
fn account_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let claims: serde_json::Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?).ok()?;
    ["email", "preferred_username", "name"]
        .iter()
        .find_map(|k| claims.get(*k).and_then(|v| v.as_str()).filter(|v| !v.is_empty()).map(str::to_owned))
}

pub(crate) fn form_encode(params: &[(&str, &str)]) -> String {
    params.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&")
}

pub(crate) fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn form_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_the_rfc_example() {
        // RFC 7636, appendix B.
        assert_eq!(pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn reads_the_account_from_an_id_token() {
        let claims = URL_SAFE_NO_PAD.encode(br#"{"sub":"1","email":"me@example.com","name":"Me"}"#);
        assert_eq!(account_from_id_token(&format!("h.{claims}.s")), Some("me@example.com".into()));
        let ms = URL_SAFE_NO_PAD.encode(br#"{"preferred_username":"me@outlook.com"}"#);
        assert_eq!(account_from_id_token(&format!("h.{ms}.s")), Some("me@outlook.com".into()));
        assert_eq!(account_from_id_token("garbage"), None);
    }

    #[test]
    fn encodes_and_decodes_form_values() {
        assert_eq!(form_encode(&[("scope", "openid email"), ("x", "a/b")]), "scope=openid%20email&x=a%2Fb");
        assert_eq!(form_decode("4%2F0Ab_c+d%3D"), "4/0Ab_c d=");
        assert_eq!(form_decode("100%"), "100%");
    }

    fn redirect(listener: &TcpListener, path: &str) -> String {
        let port = listener.local_addr().unwrap().port();
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut reply = String::new();
        let _ = std::io::Read::read_to_string(&mut s, &mut reply);
        reply
    }

    #[test]
    fn catches_the_redirect_and_checks_its_state() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let cancel = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let waiting = scope.spawn(|| wait_for_code(&listener, "s1", &cancel));
            assert!(redirect(&listener, "/favicon.ico").starts_with("HTTP/1.1 404"));
            assert!(redirect(&listener, "/?code=4%2Fabc&state=s1&scope=x").contains("You're signed in"));
            assert_eq!(waiting.join().unwrap().unwrap(), "4/abc");
        });

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        std::thread::scope(|scope| {
            let waiting = scope.spawn(|| wait_for_code(&listener, "s1", &cancel));
            redirect(&listener, "/?code=x&state=forged");
            assert!(waiting.join().unwrap().is_err(), "a mismatched state is refused");
        });

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        std::thread::scope(|scope| {
            let waiting = scope.spawn(|| wait_for_code(&listener, "s1", &cancel));
            redirect(&listener, "/?error=access_denied&state=s1");
            assert_eq!(waiting.join().unwrap().unwrap_err().to_string(), "Sign-in was cancelled.");
        });
    }

    // Talks to the real services with a made-up client; run with --ignored when online.
    #[test]
    #[ignore]
    fn real_token_endpoints_reject_unknown_clients_readably() {
        for app in [OAuthApp::google("not-a-real-client".into(), Some("x".into())), OAuthApp::microsoft("00000000-0000-0000-0000-000000000000".into())] {
            let form = [("client_id", app.client_id.as_str()), ("grant_type", "refresh_token"), ("refresh_token", "nope")];
            match post_token(&app, &form) {
                Err(RemoteError::Auth(message)) => {
                    assert!(!message.is_empty() && !message.contains('{'), "{message}");
                    println!("{}: {message}", app.token_url);
                }
                other => panic!("expected a sign-in error, got {:?}", other.map(|t| t.access_token)),
            }
        }
    }

    #[test]
    fn cancelling_stops_the_wait() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let cancel = AtomicBool::new(true);
        assert!(wait_for_code(&listener, "s", &cancel).is_err());
    }
}
