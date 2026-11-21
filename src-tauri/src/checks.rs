use std::io::Read;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;

/// What's worth knowing about a file before handing it in.
#[derive(Debug, Clone, Serialize)]
pub struct FileCheck {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    /// Programs that have it open, whose unsaved changes wouldn't be in what's handed in.
    pub open_in: Vec<String>,
    /// Something wrong with the file itself, such as being empty or not what its name says.
    pub problem: Option<String>,
    /// It's exactly what was handed in last time under the same name.
    pub same_as_last: bool,
}

/// Looks a file over before it's handed in.
pub fn check_file(path: &Path, last_sha: Option<&str>) -> FileCheck {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    let meta = std::fs::metadata(path);
    let (size, modified) = match &meta {
        Ok(m) => (m.len(), m.modified().ok().map(DateTime::<Utc>::from)),
        Err(_) => (0, None),
    };
    let problem = match &meta {
        Err(_) => Some("It isn't there any more.".to_owned()),
        Ok(m) if m.is_dir() => Some("It's a folder, not a file.".to_owned()),
        Ok(_) if size == 0 => Some("It's empty.".to_owned()),
        Ok(_) => misnamed(path),
    };
    let same_as_last = problem.is_none()
        && last_sha.is_some_and(|sha| multitasker_deliverables::hash_file(path).ok().as_deref() == Some(sha));
    FileCheck {
        path: path.display().to_string(),
        name,
        size,
        modified,
        open_in: if meta.is_ok() { open_in(path) } else { Vec::new() },
        problem,
        same_as_last,
    }
}

/// The kinds of file whose first bytes say what they really are.
const SIGNATURES: &[(&[&str], &str, fn(&[u8]) -> bool)] = &[
    (&["pdf"], "PDF", |b| b.starts_with(b"%PDF")),
    (&["docx", "xlsx", "pptx", "zip", "odt", "ods", "odp", "epub"], "document", |b| b.starts_with(b"PK\x03\x04")),
    (&["png"], "PNG image", |b| b.starts_with(b"\x89PNG")),
    (&["jpg", "jpeg"], "JPEG image", |b| b.starts_with(&[0xFF, 0xD8, 0xFF])),
    (&["gif"], "GIF image", |b| b.starts_with(b"GIF8")),
    (&["mp4", "mov", "m4a", "m4v"], "video", |b| b.get(4..8) == Some(b"ftyp")),
    (&["webm", "mkv"], "video", |b| b.starts_with(&[0x1A, 0x45, 0xDF, 0xA3])),
];

/// Why a file isn't what its name says, if it isn't: a damaged or renamed file would hand in nothing usable.
fn misnamed(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    let (_, kind, matches) = SIGNATURES.iter().find(|(exts, _, _)| exts.contains(&ext.as_str()))?;
    let mut head = [0u8; 16];
    let read = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).ok()?;
    (!matches(&head[..read])).then(|| format!("It doesn't look like a real {kind}; it may be damaged or renamed."))
}

/// Programs holding the file open, by name ("Microsoft Word").
#[cfg(windows)]
fn open_in(path: &Path) -> Vec<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::RestartManager::{
        RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY, RM_PROCESS_INFO,
    };

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut names = Vec::new();
    unsafe {
        let mut session = 0u32;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        if RmStartSession(&mut session, 0, key.as_mut_ptr()) != 0 {
            return office_lock(path);
        }
        let files = [wide.as_ptr()];
        if RmRegisterResources(session, 1, files.as_ptr(), 0, std::ptr::null(), 0, std::ptr::null()) == 0 {
            let (mut needed, mut count, mut reasons) = (0u32, 0u32, 0u32);
            let mut list: Vec<RM_PROCESS_INFO> = Vec::new();
            // The first call says how many there are; a program opening it in between just means asking again.
            for _ in 0..3 {
                list.resize_with(needed as usize, || std::mem::zeroed());
                count = needed;
                let status = RmGetList(session, &mut needed, &mut count, list.as_mut_ptr(), &mut reasons);
                if status != 234 {
                    if status != 0 {
                        count = 0;
                    }
                    break;
                }
            }
            for p in list.iter().take(count as usize) {
                let len = p.strAppName.iter().position(|&c| c == 0).unwrap_or(p.strAppName.len());
                let name = String::from_utf16_lossy(&p.strAppName[..len]);
                if !name.is_empty() && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        RmEndSession(session);
    }
    if names.is_empty() {
        office_lock(path)
    } else {
        names
    }
}

#[cfg(not(windows))]
fn open_in(path: &Path) -> Vec<String> {
    office_lock(path)
}

/// Office leaves a "~$" owner file beside a document while it's open, even where it holds no lock on the file itself.
fn office_lock(path: &Path) -> Vec<String> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().map(|n| n.to_string_lossy().into_owned())) else {
        return Vec::new();
    };
    let short: String = name.chars().skip(2).collect();
    let open = [format!("~${name}"), format!("~${short}")].iter().any(|owner| dir.join(owner).exists());
    if open {
        vec!["Microsoft Office".to_owned()]
    } else {
        Vec::new()
    }
}

/// Whether a link opens for someone else, as far as can be told without their sign-in.
#[derive(Debug, Clone, Serialize)]
pub struct LinkCheck {
    pub url: String,
    pub title: Option<String>,
    /// `ok`, `sign_in` (others must sign in to see it), `broken` (gone or erroring), `unreachable` or `unchecked`.
    pub verdict: &'static str,
    pub message: String,
}

/// Where sites send visitors who need to sign in first.
const SIGN_IN_HOSTS: &[&str] = &[
    "accounts.google.com",
    "login.microsoftonline.com",
    "login.live.com",
    "login.microsoft.com",
    "account.box.com",
    "www.dropbox.com/login",
    "github.com/login",
    "gitlab.com/users/sign_in",
    "www.notion.so/login",
    "auth.atlassian.com",
    "id.atlassian.com",
];

fn needs_sign_in(url: &str) -> bool {
    let bare = url.trim_start_matches("https://").trim_start_matches("http://");
    let lower = bare.to_lowercase();
    SIGN_IN_HOSTS.iter().any(|h| lower.starts_with(h))
        || ["/login", "/signin", "/sign-in", "/sign_in", "servicelogin", "/oauth2/authorize"].iter().any(|p| lower.contains(p))
}

/// Opens a link the way someone without your sign-in would, and says whether it worked.
pub fn check_link(url: &str) -> LinkCheck {
    use ureq::ResponseExt;

    let url = url.trim();
    let url = if url.contains("://") { url.to_owned() } else { format!("https://{url}") };
    let done = |verdict, message: &str, title| LinkCheck { url: url.clone(), title, verdict, message: message.to_owned() };
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return done("broken", "Only web links (http and https) can be handed in.", None);
    }

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(10)
        .save_redirect_history(true)
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) MultiTasker link check")
        .build()
        .into();
    let mut response = match agent.get(&url).call() {
        Ok(r) => r,
        Err(e) => return done("unreachable", &format!("It didn't open: {e}."), None),
    };
    let status = response.status().as_u16();
    let visited: Vec<String> = match response.get_redirect_history() {
        Some(history) => history.iter().map(|u| u.to_string()).collect(),
        None => vec![response.get_uri().to_string()],
    };
    let html = response.body_mut().with_config().limit(512 * 1024).read_to_string().unwrap_or_default();
    let title = page_title(&html);

    if visited.iter().skip(1).any(|u| needs_sign_in(u)) || status == 401 || status == 403 {
        return done(
            "sign_in",
            "Others have to sign in to open it. Check its sharing settings let anyone with the link see it.",
            title,
        );
    }
    // Some sites turn away anything that isn't a person's browser, so the page says nothing about who can see it.
    let blocked = title.as_deref().is_some_and(|t| {
        let t = t.to_lowercase();
        ["request has been blocked", "access denied", "just a moment", "attention required", "are you a robot", "captcha"]
            .iter()
            .any(|p| t.contains(p))
    });
    if blocked || status == 429 {
        return done(
            "unchecked",
            "The site wouldn't let MultiTasker check it. Open it in a private window to be sure others can see it.",
            None,
        );
    }
    match status {
        200..=299 => done("ok", "It opens for anyone with the link.", title),
        404 | 410 => done("broken", "That page doesn't exist (it may have moved or been deleted).", title),
        500..=599 => done("broken", &format!("The site had an error ({status}). Try again later."), title),
        _ => done("broken", &format!("The site answered with an unexpected status ({status})."), title),
    }
}

fn page_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let end = lower[open_end..].find("</title>")? + open_end;
    let title = html[open_end..end]
        .replace("&amp;", "&")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

/// A Windows internet shortcut, which is how a handed-in link is kept: it opens in the browser, and copies and syncs like any file.
pub fn shortcut(url: &str) -> Vec<u8> {
    format!("[InternetShortcut]\r\nURL={url}\r\n").into_bytes()
}

/// A file name for a link's shortcut, from its page title or address.
pub fn shortcut_name(url: &str, title: Option<&str>) -> String {
    let fallback = url.split("://").nth(1).unwrap_or(url).trim_end_matches('/');
    let base = title.filter(|t| !t.trim().is_empty()).unwrap_or(fallback);
    let safe: String = base
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let safe: String = safe.chars().take(80).collect();
    format!("{}.url", safe.trim_end_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_sign_in_pages() {
        assert!(needs_sign_in("https://accounts.google.com/ServiceLogin?continue=https://docs.google.com/x"));
        assert!(needs_sign_in("https://login.microsoftonline.com/common/oauth2/authorize?x=1"));
        assert!(needs_sign_in("https://example.com/login?next=/doc"));
        assert!(!needs_sign_in("https://docs.google.com/document/d/abc/edit"));
    }

    #[test]
    fn reads_page_titles() {
        assert_eq!(page_title("<html><TITLE>\n  My  essay &amp; notes </TITLE>").as_deref(), Some("My essay & notes"));
        assert_eq!(page_title("<p>none</p>"), None);
    }

    #[test]
    fn names_shortcuts_safely() {
        assert_eq!(shortcut_name("https://x.com/a", Some("Essay: draft 2 / final?")), "Essay draft 2 final.url");
        assert_eq!(shortcut_name("https://x.com/a/", None), "x.com a.url");
    }

    #[test]
    #[ignore = "reaches real sites"]
    fn checks_real_links() {
        for url in [
            "https://example.com",
            "https://example.com/this-page-is-not-here",
            "https://docs.google.com/document/d/1aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/edit",
            "https://onedrive.live.com/edit?id=ABCDEF",
            "https://drive.google.com/drive/my-drive",
            "https://docs.google.com/document/u/0/",
            "https://onedrive.live.com/",
        ] {
            let c = check_link(url);
            println!("{url}\n  {} | {:?} | {}", c.verdict, c.title, c.message);
        }
    }

    #[test]
    fn notices_empty_misnamed_and_open_files() {
        let dir = std::env::temp_dir().join(format!("mt-check-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let empty = dir.join("empty.txt");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(check_file(&empty, None).problem.as_deref(), Some("It's empty."));

        let fake = dir.join("essay.pdf");
        std::fs::write(&fake, b"just some text").unwrap();
        assert!(check_file(&fake, None).problem.unwrap().contains("real PDF"));

        let real = dir.join("real.pdf");
        std::fs::write(&real, b"%PDF-1.7 ...").unwrap();
        let sha = multitasker_deliverables::hash_file(&real).unwrap();
        let checked = check_file(&real, Some(&sha));
        assert!(checked.problem.is_none() && checked.same_as_last);

        let doc = dir.join("Report.docx");
        std::fs::write(&doc, b"PK\x03\x04rest").unwrap();
        assert!(check_file(&doc, None).open_in.is_empty());
        std::fs::write(dir.join("~$port.docx"), b"owner").unwrap();
        assert_eq!(check_file(&doc, None).open_in, vec!["Microsoft Office".to_owned()]);

        // A file this test holds open is reported as open in this test's own program.
        let held = dir.join("held.txt");
        std::fs::write(&held, b"x").unwrap();
        let handle = std::fs::File::open(&held).unwrap();
        #[cfg(windows)]
        assert!(!check_file(&held, None).open_in.is_empty(), "the open handle is seen");
        drop(handle);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
