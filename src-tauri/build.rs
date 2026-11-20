fn main() {
    // Official builds bake in their Google and Microsoft app registrations; see docs/cloud-sign-in.md.
    for var in ["MULTITASKER_GOOGLE_CLIENT_ID", "MULTITASKER_GOOGLE_CLIENT_SECRET", "MULTITASKER_MICROSOFT_CLIENT_ID"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    tauri_build::build()
}
