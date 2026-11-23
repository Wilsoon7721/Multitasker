mod matcher;
mod watch;
mod window;

pub use matcher::{matches, normalize_url, Match, Observation, Strength};
pub use watch::FileWatcher;
pub use window::{
    process_modules, processes_named, app_windows, foreground_is_fullscreen, foreground_window, idle_time, is_process_running, process_alive, process_windows,
    tree_cpu_time, AppWindow, ForegroundWindow, LoadedModule, ProcessWindow, RunningProcess,
};
