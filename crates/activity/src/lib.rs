mod matcher;
mod watch;
mod window;

pub use matcher::{matches, normalize_url, Match, Observation, Strength};
pub use watch::FileWatcher;
pub use window::{app_windows, foreground_is_fullscreen, foreground_window, idle_time, is_process_running, AppWindow, ForegroundWindow};
