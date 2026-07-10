use crate::app_utils::paths::resolve_media_path;
use std::{fs, path::Path};

/// Deletes a stored media file, resolving relative paths against the app data dir.
pub fn delete_media_file(app_dir: &Path, stored_path: Option<String>) {
    let Some(path) = stored_path else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let _ = fs::remove_file(resolve_media_path(app_dir, &path));
}
