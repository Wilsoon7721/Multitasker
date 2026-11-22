use std::path::{Component, Path, PathBuf};

use multitasker_core::{Attachment, AttachmentKind};
use serde::Serialize;

/// Where an attached file or folder is on this computer: its own path, or the same place under this computer's user or
/// OneDrive folder, for a file attached on another computer whose user folder is named differently.
pub fn resolve(a: &Attachment) -> Option<PathBuf> {
    let original = PathBuf::from(&a.target);
    if original.exists() {
        return Some(original);
    }
    let rest = under_user_folder(&original)?;
    let mut tries = Vec::new();
    // A path inside OneDrive maps onto whichever OneDrive folder this computer has, whatever it's called here.
    if let Some(Component::Normal(first)) = rest.components().next() {
        if first.to_string_lossy().to_lowercase().starts_with("onedrive") {
            let inside: PathBuf = rest.components().skip(1).collect();
            for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
                if let Ok(root) = std::env::var(var) {
                    tries.push(PathBuf::from(root).join(&inside));
                }
            }
        }
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        tries.push(PathBuf::from(home).join(&rest));
    }
    tries.into_iter().find(|p| p.exists())
}

/// The part of a path after `C:\Users\<name>\`, if it's inside a user folder.
fn under_user_folder(path: &Path) -> Option<PathBuf> {
    let parts: Vec<Component> = path.components().collect();
    let users = parts.iter().position(|c| matches!(c, Component::Normal(n) if n.eq_ignore_ascii_case("Users")))?;
    let rest: PathBuf = parts.get(users + 2..)?.iter().collect();
    (!rest.as_os_str().is_empty()).then_some(rest)
}

/// Whether some program on this computer opens files like this one; folders always open in Explorer.
#[cfg(windows)]
pub fn can_open(path: &Path) -> bool {
    use windows_sys::Win32::UI::Shell::{AssocQueryStringW, ASSOCF_NOTRUNCATE, ASSOCSTR_EXECUTABLE};
    if path.is_dir() {
        return true;
    }
    let Some(ext) = path.extension() else { return true };
    let ext: Vec<u16> = format!(".{}", ext.to_string_lossy()).encode_utf16().chain(Some(0)).collect();
    let mut out = [0u16; 520];
    let mut len = out.len() as u32;
    let found = unsafe {
        AssocQueryStringW(ASSOCF_NOTRUNCATE, ASSOCSTR_EXECUTABLE, ext.as_ptr(), std::ptr::null(), out.as_mut_ptr(), &mut len)
    };
    // An unknown type still answers with Windows' own "Open with" chooser, which isn't a program for it.
    let exe = String::from_utf16_lossy(&out[..out.iter().position(|&c| c == 0).unwrap_or(0)]).to_lowercase();
    found == 0 && !exe.is_empty() && !exe.ends_with("openwith.exe") && !exe.ends_with("rundll32.exe")
}

#[cfg(not(windows))]
pub fn can_open(_path: &Path) -> bool {
    true
}

/// What a task can open on this computer, and what it can't, with why.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Openable {
    pub files: usize,
    pub pages: usize,
    pub unavailable: Vec<String>,
}

/// Sorts a task's attachments into what can open here: files and folders found here with a program to open them, and
/// pages. Apps are left out; one started without its file rarely puts you back where you were.
pub fn openable(attachments: &[Attachment]) -> (Openable, Vec<PathBuf>) {
    let mut report = Openable::default();
    let mut paths = Vec::new();
    for a in attachments {
        match a.kind {
            AttachmentKind::Link => report.pages += 1,
            AttachmentKind::App => {}
            _ => match resolve(a) {
                Some(path) if can_open(&path) => {
                    report.files += 1;
                    paths.push(path);
                }
                Some(path) => {
                    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                    report.unavailable.push(format!("Nothing on this computer opens .{ext} files, like {}.", a.name));
                }
                None => report.unavailable.push(format!("{} isn't on this computer.", a.name)),
            },
        }
    }
    (report, paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_user_folder_part_of_a_path() {
        assert_eq!(
            under_user_folder(Path::new(r"C:\Users\laptop-user\OneDrive\School\essay.docx")),
            Some(PathBuf::from(r"OneDrive\School\essay.docx"))
        );
        assert_eq!(under_user_folder(Path::new(r"D:\Projects\app")), None);
        assert_eq!(under_user_folder(Path::new(r"C:\Users\someone")), None);
    }

    #[test]
    fn maps_a_file_from_another_users_folder_onto_this_one() {
        let home = std::env::var("USERPROFILE").unwrap();
        let dir = PathBuf::from(&home).join(format!("mt-here-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        let elsewhere = format!(r"C:\Users\someone-else\{}\notes.txt", dir.file_name().unwrap().to_string_lossy());
        let a = Attachment {
            id: "a".into(),
            provider: "sqlite".into(),
            task_id: "t".into(),
            kind: AttachmentKind::Document,
            name: "notes.txt".into(),
            target: elsewhere,
            device: Some("laptop".into()),
            added_at: chrono::Utc::now(),
        };
        assert_eq!(resolve(&a), Some(dir.join("notes.txt")));
        let (report, paths) = openable(std::slice::from_ref(&a));
        assert_eq!((report.files, paths.len()), (1, 1), "a .txt file has a program to open it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn knows_whether_a_program_opens_a_file_type() {
        assert!(can_open(Path::new(r"C:\anything\notes.txt")));
        assert!(!can_open(Path::new(r"C:\anything\file.multitasker-unknown-type")));
    }
}
