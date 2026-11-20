use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{DeviceId, Error, Result, TaskId};

/// What an attachment points at. Files are kept as references (their path), never copied in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Document,
    Pdf,
    Image,
    Video,
    Folder,
    File,
    Link,
}

impl AttachmentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AttachmentKind::Document => "document",
            AttachmentKind::Pdf => "pdf",
            AttachmentKind::Image => "image",
            AttachmentKind::Video => "video",
            AttachmentKind::Folder => "folder",
            AttachmentKind::File => "file",
            AttachmentKind::Link => "link",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "document" => AttachmentKind::Document,
            "pdf" => AttachmentKind::Pdf,
            "image" => AttachmentKind::Image,
            "video" => AttachmentKind::Video,
            "folder" => AttachmentKind::Folder,
            "file" => AttachmentKind::File,
            "link" => AttachmentKind::Link,
            other => return Err(Error::Invalid(format!("unknown attachment kind '{other}'"))),
        })
    }

    /// Guesses a file's kind from its extension.
    pub fn of_file(path: &Path) -> Self {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        match ext.as_str() {
            "pdf" => AttachmentKind::Pdf,
            "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "pages" | "xls" | "xlsx" | "ods" | "csv" | "ppt"
            | "pptx" | "odp" | "key" | "numbers" => AttachmentKind::Document,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "heic" | "heif" | "tif" | "tiff" | "avif" => {
                AttachmentKind::Image
            }
            "mp4" | "mov" | "mkv" | "avi" | "webm" | "wmv" | "m4v" | "flv" => AttachmentKind::Video,
            _ => AttachmentKind::File,
        }
    }
}

/// A file, folder or link attached to a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub provider: String,
    pub task_id: TaskId,
    pub kind: AttachmentKind,
    /// Shown in the UI: the file name, or the link's label or host.
    pub name: String,
    /// The absolute path, or the URL.
    pub target: String,
    /// The computer a file path belongs to; `None` for links, which work anywhere.
    pub device: Option<DeviceId>,
    pub added_at: DateTime<Utc>,
}

/// An attachment as the UI asks for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NewAttachment {
    /// A file or folder on this computer.
    Path { path: String },
    Link {
        url: String,
        #[serde(default)]
        name: Option<String>,
    },
}

impl Attachment {
    /// Checks and builds an attachment. File paths must exist on this computer.
    pub fn new(provider: &str, task_id: &TaskId, device: &DeviceId, new: NewAttachment) -> Result<Self> {
        let (kind, name, target, device) = match new {
            NewAttachment::Path { path } => {
                let p = Path::new(path.trim());
                if !p.is_absolute() {
                    return Err(Error::Invalid(format!("'{path}' is not a full file path.")));
                }
                let meta = std::fs::metadata(p).map_err(|_| Error::Invalid(format!("Can't find '{}'.", p.display())))?;
                let kind = if meta.is_dir() { AttachmentKind::Folder } else { AttachmentKind::of_file(p) };
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
                (kind, name, p.display().to_string(), Some(device.clone()))
            }
            NewAttachment::Link { url, name } => {
                let url = normalize_url(&url)?;
                let name = crate::clean(name).unwrap_or_else(|| link_label(&url));
                (AttachmentKind::Link, name, url, None)
            }
        };

        Ok(Attachment {
            id: uuid::Uuid::new_v4().to_string(),
            provider: provider.to_owned(),
            task_id: task_id.clone(),
            kind,
            name,
            target,
            device,
            added_at: Utc::now(),
        })
    }
}

/// Accepts `http(s)://…` links, adding `https://` to bare ones like `example.com/page`.
pub fn normalize_url(raw: &str) -> Result<String> {
    let raw = raw.trim();
    let invalid = || Error::Invalid(format!("'{raw}' doesn't look like a web link."));

    let url = match raw.split_once("://") {
        Some((scheme, _)) if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") => raw.to_owned(),
        Some(_) => return Err(invalid()),
        None => format!("https://{raw}"),
    };

    let host = url.split_once("://").map(|(_, rest)| rest).unwrap_or_default();
    let host = host.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() || host.contains(char::is_whitespace) || !(host.contains('.') || host.starts_with("localhost")) {
        return Err(invalid());
    }
    Ok(url)
}

/// "docs.google.com/document/…" → "docs.google.com".
fn link_label(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.strip_prefix("www.").unwrap_or(host).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_files_by_extension() {
        assert_eq!(AttachmentKind::of_file(Path::new("notes.DOCX")), AttachmentKind::Document);
        assert_eq!(AttachmentKind::of_file(Path::new("paper.pdf")), AttachmentKind::Pdf);
        assert_eq!(AttachmentKind::of_file(Path::new("shot.png")), AttachmentKind::Image);
        assert_eq!(AttachmentKind::of_file(Path::new("demo.mp4")), AttachmentKind::Video);
        assert_eq!(AttachmentKind::of_file(Path::new("archive.zip")), AttachmentKind::File);
        assert_eq!(AttachmentKind::of_file(Path::new("Makefile")), AttachmentKind::File);
    }

    #[test]
    fn normalizes_links() {
        assert_eq!(normalize_url("example.com/a").unwrap(), "https://example.com/a");
        assert_eq!(normalize_url(" http://localhost:3000 ").unwrap(), "http://localhost:3000");
        assert!(normalize_url("ftp://example.com").is_err());
        assert!(normalize_url("javascript:alert(1)").is_err());
        assert!(normalize_url("not a link").is_err());
        assert!(normalize_url("").is_err());
    }

    #[test]
    fn links_are_named_by_host_unless_labelled() {
        let device = "pc".to_string();
        let link = |name: Option<&str>| {
            Attachment::new("p", &"t".into(), &device, NewAttachment::Link {
                url: "https://www.claude.ai/chat/123".into(),
                name: name.map(Into::into),
            })
            .unwrap()
        };
        assert_eq!(link(None).name, "claude.ai");
        assert_eq!(link(Some(" GS notes chat ")).name, "GS notes chat");
        assert_eq!(link(None).device, None);
    }

    #[test]
    fn paths_must_exist_and_be_absolute() {
        let device = "pc".to_string();
        let task = "t".to_string();
        assert!(Attachment::new("p", &task, &device, NewAttachment::Path { path: "relative.txt".into() }).is_err());

        let dir = std::env::temp_dir();
        let folder = Attachment::new("p", &task, &device, NewAttachment::Path { path: dir.display().to_string() }).unwrap();
        assert_eq!(folder.kind, AttachmentKind::Folder);
        assert_eq!(folder.device.as_deref(), Some("pc"));

        let missing = dir.join("multitasker-definitely-missing.pdf");
        assert!(Attachment::new("p", &task, &device, NewAttachment::Path { path: missing.display().to_string() }).is_err());
    }
}
