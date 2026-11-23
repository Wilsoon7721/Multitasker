use std::sync::Arc;

use multitasker_sync::{OAuthApp, TokenSource};
use serde::Serialize;

use crate::settings::SyncTarget;

const KEYRING_SERVICE: &str = "MultiTasker";

/// A service people can sign in to for sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cloud {
    Google,
    Microsoft,
}

impl Cloud {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "google" => Ok(Cloud::Google),
            "microsoft" => Ok(Cloud::Microsoft),
            other => Err(format!("Unknown sign-in service '{other}'.")),
        }
    }

    pub fn of(target: &SyncTarget) -> Option<(Self, Option<&str>)> {
        match target {
            SyncTarget::GoogleDrive { client_id, .. } => Some((Cloud::Google, client_id.as_deref())),
            SyncTarget::OneDrive { client_id, .. } => Some((Cloud::Microsoft, client_id.as_deref())),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Cloud::Google => "google",
            Cloud::Microsoft => "microsoft",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Cloud::Google => "Google",
            Cloud::Microsoft => "Microsoft",
        }
    }

    /// The client ID this build was made with, if any.
    fn built_in_client_id(self) -> Option<&'static str> {
        match self {
            Cloud::Google => option_env!("MULTITASKER_GOOGLE_CLIENT_ID"),
            Cloud::Microsoft => option_env!("MULTITASKER_MICROSOFT_CLIENT_ID"),
        }
        .filter(|id| !id.is_empty())
    }

    /// The registration to sign in with: one entered in Settings wins over the one this build was made with.
    pub fn app(self, client_id: Option<&str>) -> Result<OAuthApp, String> {
        let id = client_id.filter(|id| !id.is_empty()).or(self.built_in_client_id()).ok_or_else(|| {
            format!("This build has no {} app registration; enter a client ID from your own app registration.", self.label())
        })?;
        Ok(match self {
            Cloud::Google => {
                let secret = if client_id.is_some_and(|c| !c.is_empty()) {
                    secret(&format!("oauth-client:{}", self.key()))
                } else {
                    option_env!("MULTITASKER_GOOGLE_CLIENT_SECRET").map(str::to_owned)
                };
                OAuthApp::google(id.to_owned(), secret)
            }
            Cloud::Microsoft => OAuthApp::microsoft(id.to_owned()),
        })
    }

    /// Keeps a hand-entered Google client secret in the credential store rather than settings.json.
    pub fn save_client_secret(self, value: &str) -> Result<(), String> {
        save_secret(&format!("oauth-client:{}", self.key()), value)
    }

    pub fn save_refresh_token(self, token: &str) -> Result<(), String> {
        save_secret(&format!("oauth:{}", self.key()), token)
    }

    /// A token source from the saved sign-in, keeping rotated refresh tokens saved.
    pub fn tokens(self, app: OAuthApp) -> Result<Arc<TokenSource>, String> {
        let refresh = secret(&format!("oauth:{}", self.key()))
            .ok_or_else(|| format!("Not signed in to {} on this computer; sign in again in Settings.", self.label()))?;
        Ok(Arc::new(TokenSource::new(app, refresh, move |rotated| {
            let _ = self.save_refresh_token(rotated);
        })))
    }

    /// Forgets the sign-in on this computer.
    pub fn sign_out(self) {
        for user in [format!("oauth:{}", self.key()), format!("oauth-client:{}", self.key())] {
            if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, &user) {
                let _ = entry.delete_credential();
            }
        }
    }
}

fn secret(user: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, user).ok()?.get_password().ok()
}

fn save_secret(user: &str, value: &str) -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, user)
        .and_then(|e| e.set_password(value))
        .map_err(|e| format!("Can't save to the credential store: {e}"))
}

/// Which services this build can sign in to without a client ID being entered.
#[derive(Debug, Clone, Serialize)]
pub struct BuiltInClients {
    pub google: bool,
    pub microsoft: bool,
}

pub fn built_in_clients() -> BuiltInClients {
    BuiltInClients {
        google: Cloud::Google.built_in_client_id().is_some(),
        microsoft: Cloud::Microsoft.built_in_client_id().is_some(),
    }
}
