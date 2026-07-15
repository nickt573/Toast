// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    undo_snap_env_rewrites();

    toast_lib::run()
}

/// Snap packages (VS Code is the common offender) rewrite GTK/GLib env vars and every child inherits
/// them. WebKit's media subprocesses broke on this and audio silently stopped decoding.
/// Pre-rewrite values live in <VAR>_VSCODE_SNAP_ORIG, restore before GTK initializes (unset if orig was empty).
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
