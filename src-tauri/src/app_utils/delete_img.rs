use std::{fs, path::Path};

/// Deletes a stored media file, resolving relative paths against the app data dir.
pub fn delete_media_file(app_dir: &Path, stored_path: Option<String>) {
    let Some(path) = stored_path else {
        return;
    };
    let full_path = if Path::new(&path).is_absolute() {
        Path::new(&path).to_path_buf()
    } else {
        app_dir.join(path)
    };
    let _ = fs::remove_file(full_path);
}
