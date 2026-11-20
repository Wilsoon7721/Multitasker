use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::oauth::{encode, TokenSource};
use crate::remote::{Remote, RemoteError};

/// A signed-in HTTP client: adds the bearer token and, if it's been rejected, renews it and retries once.
struct Authed {
    tokens: Arc<TokenSource>,
    agent: ureq::Agent,
}

impl Authed {
    fn new(tokens: Arc<TokenSource>) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();
        Self { tokens, agent }
    }

    fn send(&self, method: &str, url: &str, body: Option<(&str, &[u8])>) -> Result<(u16, Vec<u8>), RemoteError> {
        for attempt in 0..2 {
            let auth = format!("Bearer {}", self.tokens.access_token()?);
            let result = match (method, body) {
                ("GET", _) => self.agent.get(url).header("authorization", &auth).call(),
                ("DELETE", _) => self.agent.delete(url).header("authorization", &auth).call(),
                (_, Some((content_type, data))) => {
                    let request = match method {
                        "PUT" => self.agent.put(url),
                        "PATCH" => self.agent.patch(url),
                        _ => self.agent.post(url),
                    };
                    request.header("authorization", &auth).header("content-type", content_type).send(data)
                }
                (_, None) => self.agent.post(url).header("authorization", &auth).send_empty(),
            };
            let mut response = result.map_err(|e| RemoteError::Io(e.to_string()))?;
            let status = response.status().as_u16();
            if status == 401 && attempt == 0 {
                self.tokens.invalidate();
                continue;
            }
            let bytes = response.body_mut().read_to_vec().map_err(|e| RemoteError::Io(e.to_string()))?;
            if status == 401 {
                return Err(RemoteError::Auth("The sign-in has expired; sign in again in Settings.".into()));
            }
            return Ok((status, bytes));
        }
        unreachable!("the loop returns on its second attempt")
    }
}

/// The service's own error message from a JSON error body, or a generic one.
fn failure(status: u16, body: &[u8]) -> RemoteError {
    let message = serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|v| {
        let error = v.get("error")?;
        error.get("message").and_then(|m| m.as_str()).or_else(|| error.as_str()).map(str::to_owned)
    });
    RemoteError::Io(message.unwrap_or_else(|| format!("The storage service answered {status}.")))
}

const DRIVE: &str = "https://www.googleapis.com/drive/v3/files";
const DRIVE_UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";

/// Google Drive's hidden app-data folder; Drive addresses files by id, so each key is one file named after the key.
pub struct GoogleDriveRemote {
    http: Authed,
    ids: Mutex<HashMap<String, String>>,
}

impl GoogleDriveRemote {
    pub fn new(tokens: Arc<TokenSource>) -> Self {
        Self { http: Authed::new(tokens), ids: Mutex::new(HashMap::new()) }
    }

    fn ids(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.ids.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn find(&self, key: &str) -> Result<Option<String>, RemoteError> {
        if let Some(id) = self.ids().get(key) {
            return Ok(Some(id.clone()));
        }
        let q = format!("name = '{}'", key.replace('\\', "\\\\").replace('\'', "\\'"));
        let url = format!("{DRIVE}?spaces=appDataFolder&fields=files(id,name)&q={}", encode(&q));
        let (status, body) = self.http.send("GET", &url, None)?;
        if status != 200 {
            return Err(failure(status, &body));
        }
        let id = parse_files(&body)?.into_iter().next().map(|(_, id)| id);
        if let Some(id) = &id {
            self.ids().insert(key.to_owned(), id.clone());
        }
        Ok(id)
    }
}

fn parse_files(body: &[u8]) -> Result<Vec<(String, String)>, RemoteError> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| RemoteError::Io(e.to_string()))?;
    Ok(v.get("files")
        .and_then(|f| f.as_array())
        .into_iter()
        .flatten()
        .filter_map(|f| Some((f.get("name")?.as_str()?.to_owned(), f.get("id")?.as_str()?.to_owned())))
        .collect())
}

/// Drive's metadata-plus-content upload body.
fn multipart(boundary: &str, name: &str, data: &[u8]) -> Vec<u8> {
    let metadata = serde_json::json!({ "name": name, "parents": ["appDataFolder"] });
    let mut body = format!(
        "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--").as_bytes());
    body
}

impl Remote for GoogleDriveRemote {
    fn list(&self, prefix: &str) -> Result<Vec<String>, RemoteError> {
        let mut keys = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut url = format!("{DRIVE}?spaces=appDataFolder&pageSize=1000&fields=nextPageToken,files(id,name)");
            if let Some(token) = &page {
                url.push_str(&format!("&pageToken={}", encode(token)));
            }
            let (status, body) = self.http.send("GET", &url, None)?;
            if status != 200 {
                return Err(failure(status, &body));
            }
            let files = parse_files(&body)?;
            {
                let mut ids = self.ids();
                for (name, id) in &files {
                    ids.insert(name.clone(), id.clone());
                }
            }
            keys.extend(files.into_iter().map(|(name, _)| name).filter(|n| n.starts_with(prefix)));
            page = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("nextPageToken")?.as_str().map(str::to_owned));
            if page.is_none() {
                return Ok(keys);
            }
        }
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, RemoteError> {
        let Some(id) = self.find(key)? else { return Ok(None) };
        match self.http.send("GET", &format!("{DRIVE}/{id}?alt=media"), None)? {
            (200, body) => Ok(Some(body)),
            (404, _) => {
                self.ids().remove(key);
                Ok(None)
            }
            (status, body) => Err(failure(status, &body)),
        }
    }

    fn put(&self, key: &str, data: &[u8]) -> Result<(), RemoteError> {
        if let Some(id) = self.find(key)? {
            let url = format!("{DRIVE_UPLOAD}/{id}?uploadType=media");
            return match self.http.send("PATCH", &url, Some(("application/octet-stream", data)))? {
                (200, _) => Ok(()),
                (status, body) => Err(failure(status, &body)),
            };
        }
        let boundary = format!("multitasker-{}", uuid::Uuid::new_v4().simple());
        let content_type = format!("multipart/related; boundary={boundary}");
        let body = multipart(&boundary, key, data);
        let url = format!("{DRIVE_UPLOAD}?uploadType=multipart&fields=id");
        match self.http.send("POST", &url, Some((&content_type, &body)))? {
            (200, response) => {
                let id = serde_json::from_slice::<serde_json::Value>(&response)
                    .ok()
                    .and_then(|v| v.get("id")?.as_str().map(str::to_owned));
                if let Some(id) = id {
                    self.ids().insert(key.to_owned(), id);
                }
                Ok(())
            }
            (status, body) => Err(failure(status, &body)),
        }
    }
}

const APP_FOLDER: &str = "https://graph.microsoft.com/v1.0/me/drive/special/approot";

/// OneDrive's app folder (Apps/<app name>): the app can't see anything else in the person's OneDrive.
pub struct OneDriveRemote {
    http: Authed,
}

impl OneDriveRemote {
    pub fn new(tokens: Arc<TokenSource>) -> Self {
        Self { http: Authed::new(tokens) }
    }

    /// Lists a folder's files, recursing into subfolders; a folder that doesn't exist yet is empty.
    fn walk(&self, folder: &str, keys: &mut Vec<String>) -> Result<(), RemoteError> {
        let mut url = Some(format!("{APP_FOLDER}:/{}:/children?$select=name,folder,file&$top=999", path(folder)));
        while let Some(next) = url.take() {
            let (status, body) = self.http.send("GET", &next, None)?;
            match status {
                200 => {}
                404 => return Ok(()),
                _ => return Err(failure(status, &body)),
            }
            let v: serde_json::Value = serde_json::from_slice(&body).map_err(|e| RemoteError::Io(e.to_string()))?;
            for item in v.get("value").and_then(|x| x.as_array()).into_iter().flatten() {
                let Some(name) = item.get("name").and_then(|n| n.as_str()) else { continue };
                let child = format!("{folder}/{name}");
                if item.get("folder").is_some() {
                    self.walk(&child, keys)?;
                } else {
                    keys.push(child);
                }
            }
            url = v.get("@odata.nextLink").and_then(|n| n.as_str()).map(str::to_owned);
        }
        Ok(())
    }
}

/// Percent-encodes each path segment, keeping the slashes between them.
fn path(key: &str) -> String {
    key.split('/').map(encode).collect::<Vec<_>>().join("/")
}

impl Remote for OneDriveRemote {
    fn list(&self, prefix: &str) -> Result<Vec<String>, RemoteError> {
        let mut keys = Vec::new();
        self.walk(prefix.trim_end_matches('/'), &mut keys)?;
        Ok(keys)
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, RemoteError> {
        // Graph answers with a redirect to a pre-signed download URL, which the client follows without the token.
        match self.http.send("GET", &format!("{APP_FOLDER}:/{}:/content", path(key)), None)? {
            (200, body) => Ok(Some(body)),
            (404, _) => Ok(None),
            (status, body) => Err(failure(status, &body)),
        }
    }

    fn put(&self, key: &str, data: &[u8]) -> Result<(), RemoteError> {
        let url = format!("{APP_FOLDER}:/{}:/content", path(key));
        match self.http.send("PUT", &url, Some(("application/octet-stream", data)))? {
            (200 | 201, _) => Ok(()),
            (status, body) => Err(failure(status, &body)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_drive_multipart_uploads() {
        let body = String::from_utf8(multipart("b", "multitasker/v1/x.json", b"[1]")).unwrap();
        assert!(body.starts_with("--b\r\nContent-Type: application/json"));
        assert!(body.contains(r#""name":"multitasker/v1/x.json""#) && body.contains(r#""parents":["appDataFolder"]"#));
        assert!(body.ends_with("\r\n\r\n[1]\r\n--b--"));
    }

    #[test]
    fn reads_drive_file_lists() {
        let files = parse_files(br#"{"files":[{"id":"1","name":"a"},{"id":"2","name":"b"},{"bad":true}]}"#).unwrap();
        assert_eq!(files, [("a".into(), "1".into()), ("b".into(), "2".into())]);
    }

    #[test]
    fn encodes_onedrive_paths_per_segment() {
        assert_eq!(path("multitasker/v1/ops/pc 1/0000000001.json"), "multitasker/v1/ops/pc%201/0000000001.json");
    }

    #[test]
    fn surfaces_service_error_messages() {
        let google = failure(403, br#"{"error":{"code":403,"message":"Rate limit exceeded"}}"#);
        assert_eq!(google.to_string(), "Rate limit exceeded");
        let graph = failure(400, br#"{"error":{"code":"invalidRequest","message":"Bad path"}}"#);
        assert_eq!(graph.to_string(), "Bad path");
        assert_eq!(failure(500, b"oops").to_string(), "The storage service answered 500.");
    }
}
