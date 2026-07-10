// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    undo_snap_env_rewrites();

    toast_lib::run()
}

/// Snap-packaged apps (VS Code being the common offender) rewrite GTK/GLib
/// env vars to point inside their snap (e.g. GTK_PATH=/snap/code/...), and
/// every child process inherits them. A GTK/WebKit process then dlopens
/// modules built against the snap's own glibc and dies with a symbol lookup
/// error — in this app WebKit's media subprocesses broke and audio silently
/// stopped decoding. The wrapper keeps each pre-rewrite value in
/// <VAR>_VSCODE_SNAP_ORIG, so restore it (unset when the original was empty)
/// before anything GTK-related initializes.
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
