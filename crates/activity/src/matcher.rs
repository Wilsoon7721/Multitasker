use std::path::{Component, Path};

use multitasker_core::{Attachment, AttachmentKind, TaskId};

/// Something seen on the computer that might belong to a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// The window in front: its program's file name (e.g. `WINWORD.EXE`) and title.
    Window { exe: String, title: String },
    /// A file that was just created or saved.
    FileSaved { path: String },
}

/// How sure a match is; stronger matches win.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strength {
    /// The task's program is in front, but it could be doing anything.
    Weak,
    /// A folder linked to the task shows in the window title.
    Medium,
    /// A file linked to the task is open or was saved.
    Strong,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub task_id: TaskId,
    pub strength: Strength,
    /// Why, for the UI: "“notes.docx” is open".
    pub reason: String,
}

/// Folders whose churn comes from tools (builds, git, installs) rather than from someone working.
const NOISY_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build", ".next", "__pycache__", ".venv", ".cache"];

/// The best match per task for one observation, strongest first.
pub fn matches(attachments: &[Attachment], observation: &Observation) -> Vec<Match> {
    let mut best: Vec<Match> = Vec::new();
    for a in attachments {
        let Some((strength, reason)) = match_one(a, observation) else { continue };
        match best.iter_mut().find(|m| m.task_id == a.task_id) {
            Some(m) if m.strength >= strength => {}
            Some(m) => *m = Match { task_id: a.task_id.clone(), strength, reason },
            None => best.push(Match { task_id: a.task_id.clone(), strength, reason }),
        }
    }
    best.sort_by(|a, b| b.strength.cmp(&a.strength));
    best
}

fn match_one(a: &Attachment, observation: &Observation) -> Option<(Strength, String)> {
    match observation {
        Observation::Window { exe, title } => match a.kind {
            AttachmentKind::Link => None,
            AttachmentKind::App => file_name(&a.target)
                .eq_ignore_ascii_case(exe)
                .then(|| (Strength::Weak, format!("{} is in front", a.name))),
            AttachmentKind::Folder => {
                contains_token(title, &a.name, 3).then(|| (Strength::Medium, format!("“{}” is open", a.name)))
            }
            _ => {
                let stem = Path::new(&a.name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                // Many apps leave the extension out of the title ("notes - Word"), so the bare name counts too.
                (contains_token(title, &a.name, 1) || contains_token(title, &stem, 4))
                    .then(|| (Strength::Strong, format!("“{}” is open", a.name)))
            }
        },
        Observation::FileSaved { path } => {
            if is_noise(path) {
                return None;
            }
            match a.kind {
                AttachmentKind::Link | AttachmentKind::App => None,
                AttachmentKind::Folder => {
                    is_inside(path, &a.target).then(|| (Strength::Strong, format!("saved in “{}”", a.name)))
                }
                _ => same_path(path, &a.target).then(|| (Strength::Strong, format!("“{}” was saved", a.name))),
            }
        }
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Whether `needle` appears in `haystack` as a whole word or phrase, ignoring case.
fn contains_token(haystack: &str, needle: &str, min_len: usize) -> bool {
    let needle = needle.trim();
    if needle.chars().count() < min_len {
        return false;
    }
    let (hay, needle) = (haystack.to_lowercase(), needle.to_lowercase());
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric());
    hay.match_indices(&needle).any(|(i, m)| !is_word(hay[..i].chars().last()) && !is_word(hay[i + m.len()..].chars().next()))
}

fn normalize(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn same_path(a: &str, b: &str) -> bool {
    normalize(a) == normalize(b)
}

fn is_inside(path: &str, folder: &str) -> bool {
    let (path, folder) = (normalize(path), normalize(folder));
    path.len() > folder.len() && path.starts_with(&folder) && path[folder.len()..].starts_with('\\')
}

/// Office lock files, temp files and tool folders aren't someone working.
fn is_noise(path: &str) -> bool {
    let name = file_name(path).to_lowercase();
    if name.starts_with("~$") || name.starts_with(".~") || name.ends_with(".tmp") || name.ends_with('~') || name.ends_with(".crdownload") {
        return true;
    }
    Path::new(path).components().any(|c| matches!(c, Component::Normal(s) if NOISY_DIRS.iter().any(|d| s.eq_ignore_ascii_case(d))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use multitasker_core::Attachment;

    fn attachment(task: &str, kind: AttachmentKind, name: &str, target: &str) -> Attachment {
        Attachment {
            id: format!("{task}-{name}"),
            provider: "sqlite".into(),
            task_id: task.into(),
            kind,
            name: name.into(),
            target: target.into(),
            device: Some("pc".into()),
            added_at: chrono::Utc::now(),
        }
    }

    fn window(exe: &str, title: &str) -> Observation {
        Observation::Window { exe: exe.into(), title: title.into() }
    }

    fn saved(path: &str) -> Observation {
        Observation::FileSaved { path: path.into() }
    }

    fn sample() -> Vec<Attachment> {
        vec![
            attachment("essay", AttachmentKind::Document, "GS essay.docx", r"C:\School\GS essay.docx"),
            attachment("essay", AttachmentKind::App, "Microsoft Word", r"C:\Office\WINWORD.EXE"),
            attachment("dash", AttachmentKind::Folder, "JavaDashboard", r"C:\Dev\JavaDashboard"),
            attachment("dash", AttachmentKind::Link, "github.com", "https://github.com"),
            attachment("slides", AttachmentKind::App, "PowerPoint", r"C:\Office\POWERPNT.EXE"),
        ]
    }

    #[test]
    fn open_files_beat_their_program() {
        let found = matches(&sample(), &window("WINWORD.EXE", "GS essay - Word"));
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].task_id.as_str(), found[0].strength), ("essay", Strength::Strong));
        assert_eq!(found[0].reason, "“GS essay.docx” is open");
    }

    #[test]
    fn programs_alone_are_weak_matches() {
        let found = matches(&sample(), &window("winword.exe", "Document1 - Word"));
        assert_eq!(found.iter().map(|m| (m.task_id.as_str(), m.strength)).collect::<Vec<_>>(), [("essay", Strength::Weak)]);
    }

    #[test]
    fn folders_match_window_titles_as_whole_words() {
        let found = matches(&sample(), &window("Code.exe", "page.jsx - JavaDashboard - Visual Studio Code"));
        assert_eq!((found[0].task_id.as_str(), found[0].strength), ("dash", Strength::Medium));
        assert!(matches(&sample(), &window("Code.exe", "MyJavaDashboards - Visual Studio Code")).is_empty());
    }

    #[test]
    fn short_names_need_the_full_file_name() {
        let tasks = vec![attachment("t", AttachmentKind::Document, "a.md", r"C:\a.md")];
        assert!(matches(&tasks, &window("notepad.exe", "a - Notepad")).is_empty());
        assert_eq!(matches(&tasks, &window("notepad.exe", "a.md - Notepad")).len(), 1);
    }

    #[test]
    fn saves_match_files_and_anything_inside_folders() {
        let found = matches(&sample(), &saved(r"c:/school/gs essay.docx"));
        assert_eq!((found[0].task_id.as_str(), found[0].reason.as_str()), ("essay", "“GS essay.docx” was saved"));

        let found = matches(&sample(), &saved(r"C:\Dev\JavaDashboard\app\page.jsx"));
        assert_eq!(found[0].task_id, "dash");
        assert!(matches(&sample(), &saved(r"C:\Dev\JavaDashboardOld\x.js")).is_empty());
    }

    #[test]
    fn tool_and_temp_files_are_ignored() {
        for path in [
            r"C:\School\~$ essay.docx",
            r"C:\Dev\JavaDashboard\node_modules\react\index.js",
            r"C:\Dev\JavaDashboard\.git\index",
            r"C:\Dev\JavaDashboard\app\page.jsx.tmp",
        ] {
            assert!(matches(&sample(), &saved(path)).is_empty(), "{path}");
        }
    }
}
