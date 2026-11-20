use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use multitasker_provider_javadashboard::{Bearer, JavaDashboard};
use multitasker_sync::{OAuthApp, Pending, TokenSource};

/// Where JavaDashboard's API lives.
pub const DASHBOARD: &str = "https://dash.wilsoon.dev";
/// WilsoonID sends the sign-in back to this app link, which Windows hands to MultiTasker (RFC 8252 §7.1).
pub const REDIRECT_URI: &str = "dev.wilsoon.multitasker:/oauth2redirect";
/// How long to wait for the person to finish signing in in their browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

const KEYRING_SERVICE: &str = "MultiTasker";
const KEYRING_USER: &str = "oauth:wilsoon";

/// The WilsoonID registration this build was made with.
pub fn app() -> Result<OAuthApp, String> {
    option_env!("WILSOON_CLIENT_ID")
        .filter(|id| !id.is_empty())
        .map(|id| OAuthApp::wilsoon(id.to_owned()))
        .ok_or_else(|| "This build has no WilsoonID registration (WILSOON_CLIENT_ID).".to_owned())
}

pub fn is_available() -> bool {
    app().is_ok()
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| format!("Can't open the credential store: {e}"))
}

pub fn save_refresh_token(token: &str) -> Result<(), String> {
    entry()?.set_password(token).map_err(|e| format!("Can't save the sign-in: {e}"))
}

pub fn sign_out() {
    if let Ok(entry) = entry() {
        let _ = entry.delete_credential();
    }
}

/// Tokens for JavaDashboard from the saved sign-in; WilsoonID rotates refresh tokens, so new ones are saved.
pub fn tokens() -> Result<Arc<TokenSource>, String> {
    let refresh = entry()?.get_password().map_err(|_| "Not signed in to WilsoonID on this computer.".to_owned())?;
    Ok(Arc::new(TokenSource::new(app()?, refresh, |rotated| {
        let _ = save_refresh_token(rotated);
    })))
}

/// Gives the JavaDashboard provider its bearer tokens.
struct Tokens(Arc<TokenSource>);

impl Bearer for Tokens {
    fn token(&self) -> Result<String, String> {
        self.0.access_token().map_err(|e| e.to_string())
    }

    fn invalidate(&self) {
        self.0.invalidate();
    }
}

pub fn provider(tokens: Arc<TokenSource>) -> JavaDashboard {
    JavaDashboard::new(DASHBOARD, Arc::new(Tokens(tokens)))
}

/// A sign-in waiting for the browser to come back through the app link.
pub struct Waiting {
    state: String,
    sender: Sender<String>,
}

/// The one sign-in that may be waiting at a time; the app-link handler delivers to it.
#[derive(Default)]
pub struct SignInSlot(Mutex<Option<Waiting>>);

impl SignInSlot {
    /// Starts waiting for `pending` to come back; replaces any sign-in that was already waiting.
    pub fn wait_for(&self, pending: &Pending) -> Receiver<String> {
        let (sender, receiver) = mpsc::channel();
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(Waiting { state: pending.state().to_owned(), sender });
        receiver
    }

    /// Hands an app-link URL to the waiting sign-in, if it's the one it was waiting for.
    pub fn deliver(&self, url: &str) -> bool {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let matches = slot.as_ref().is_some_and(|w| url.contains(&format!("state={}", w.state)) || url.contains("error="));
        if matches {
            if let Some(waiting) = slot.take() {
                let _ = waiting.sender.send(url.to_owned());
            }
        }
        matches
    }

    pub fn clear(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Waits for the app link to bring the sign-in back, checking `cancelled` as it goes.
pub fn receive(receiver: &Receiver<String>, cancelled: impl Fn() -> bool) -> Result<String, String> {
    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    loop {
        if cancelled() {
            return Err("Sign-in was cancelled.".into());
        }
        match receiver.recv_timeout(Duration::from_millis(200)) {
            Ok(url) => return Ok(url),
            Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
            Err(_) => return Err("Sign-in timed out; try again.".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_waiting_sign_in_gets_the_redirect() {
        let slot = SignInSlot::default();
        let pending = multitasker_sync::begin(&OAuthApp::wilsoon("cl_x".into()), REDIRECT_URI);
        let receiver = slot.wait_for(&pending);

        assert!(!slot.deliver("dev.wilsoon.multitasker:/oauth2redirect?code=c&state=someone-else"));
        let url = format!("dev.wilsoon.multitasker:/oauth2redirect?code=c&state={}", pending.state());
        assert!(slot.deliver(&url));
        assert_eq!(receive(&receiver, || false).unwrap(), url);
        assert!(!slot.deliver(&url), "a sign-in is delivered once");
    }

    #[test]
    fn cancelling_stops_waiting() {
        let slot = SignInSlot::default();
        let pending = multitasker_sync::begin(&OAuthApp::wilsoon("cl_x".into()), REDIRECT_URI);
        let receiver = slot.wait_for(&pending);
        assert_eq!(receive(&receiver, || true).unwrap_err(), "Sign-in was cancelled.");
    }
}
