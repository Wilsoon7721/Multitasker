use std::path::Path;

/// The app registrations official builds sign in with; see docs/cloud-sign-in.md.
const CLIENT_VARS: [&str; 3] =
    ["MULTITASKER_GOOGLE_CLIENT_ID", "MULTITASKER_GOOGLE_CLIENT_SECRET", "MULTITASKER_MICROSOFT_CLIENT_ID"];

fn main() {
    let env_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".env");
    println!("cargo:rerun-if-changed={}", env_file.display());
    let from_file = std::fs::read_to_string(&env_file).unwrap_or_default();

    for var in CLIENT_VARS {
        println!("cargo:rerun-if-env-changed={var}");
        // A real environment variable (as CI sets) wins over the .env file.
        let value = std::env::var(var).ok().filter(|v| !v.is_empty()).or_else(|| dotenv_value(&from_file, var));
        if let Some(value) = value {
            println!("cargo:rustc-env={var}={value}");
        }
    }
    tauri_build::build()
}

/// Reads `KEY=value` from .env text, ignoring comments and blank lines and stripping optional quotes.
fn dotenv_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.strip_prefix("export ").unwrap_or(line).split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().trim_matches('"').trim_matches('\'').to_owned())
        .filter(|v| !v.is_empty())
}
