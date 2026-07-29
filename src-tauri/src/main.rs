// Prevents an additional console window on Windows in release, do not remove
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    undo_snap_env_rewrites();

    toast_lib::run()
}

/// Snap packages rewrite the GTK and GLib env vars and every child inherits them, which
/// stopped WebKit decoding audio, so restore the saved originals before GTK initializes
#[cfg(target_os = "linux")]
fn undo_snap_env_rewrites() {
    let vars: Vec<(String, String)> = std::env::vars().collect();
    for (key, orig) in vars {
        let Some(name) = key.strip_suffix("_VSCODE_SNAP_ORIG") else {
            continue;
        };
        let current = std::env::var(name).unwrap_or_default();
        if current.contains("/snap/") {
            if orig.is_empty() {
                std::env::remove_var(name);
            } else {
                std::env::set_var(name, orig);
            }
        }
    }
}
