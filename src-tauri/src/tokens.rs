use multitasker_provider_notion::Notion;
use multitasker_provider_todoist::Todoist;

const KEYRING_SERVICE: &str = "MultiTasker";

/// A task list that connects with a token the person pastes in, kept in Windows Credential Manager.
#[derive(Debug, Clone, Copy)]
pub enum Service {
    Todoist,
    Notion,
}

impl Service {
    fn user(self) -> &'static str {
        match self {
            Service::Todoist => "token:todoist",
            Service::Notion => "token:notion",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Service::Todoist => "Todoist",
            Service::Notion => "Notion",
        }
    }
}

fn entry(service: Service) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, service.user()).map_err(|e| format!("Can't open the credential store: {e}"))
}

pub fn save(service: Service, token: &str) -> Result<(), String> {
    entry(service)?.set_password(token).map_err(|e| format!("Can't save the {} token: {e}", service.label()))
}

pub fn load(service: Service) -> Option<String> {
    entry(service).ok()?.get_password().ok().filter(|t| !t.trim().is_empty())
}

pub fn forget(service: Service) {
    if let Ok(entry) = entry(service) {
        let _ = entry.delete_credential();
    }
}

pub fn todoist() -> Option<Todoist> {
    load(Service::Todoist).map(Todoist::new)
}

pub fn notion(database: &str) -> Option<Notion> {
    load(Service::Notion).map(|token| Notion::new(token, database))
}
