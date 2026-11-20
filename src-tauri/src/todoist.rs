use multitasker_provider_todoist::Todoist;

const KEYRING_SERVICE: &str = "MultiTasker";
const KEYRING_USER: &str = "token:todoist";

/// Where people find their personal token in Todoist.
pub const TOKEN_PAGE: &str = "https://app.todoist.com/app/settings/integrations/developer";

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| format!("Can't open the credential store: {e}"))
}

pub fn save_token(token: &str) -> Result<(), String> {
    entry()?.set_password(token).map_err(|e| format!("Can't save the Todoist token: {e}"))
}

pub fn forget() {
    if let Ok(entry) = entry() {
        let _ = entry.delete_credential();
    }
}

/// Todoist with the saved token, if there is one.
pub fn provider() -> Option<Todoist> {
    entry().ok()?.get_password().ok().filter(|t| !t.trim().is_empty()).map(Todoist::new)
}
