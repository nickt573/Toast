use crate::app_utils::paths::{is_stored_media, to_relative};
use rusqlite::Result;
use std::path::{Path, PathBuf};

pub fn save_card_audio_file(src: Option<String>, app_dir: &Path) -> Result<Option<String>> {
    match src {
        None => Ok(None),
        Some(ref s) if s.is_empty() => Ok(None),
        Some(p) => Ok(save_card_audio_files(vec![p], app_dir)?.into_iter().next()),
    }
}

pub fn save_card_audio_files(src_paths: Vec<String>, app_dir: &Path) -> Result<Vec<String>> {
    let dest_dir = app_dir.join("cards").join("audio");
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;

    let mut result = Vec::new();

    for src in src_paths {
        if src.is_empty() {
            continue;
        }

        // Already stored (relative or a legacy absolute form), keep normalized
        if is_stored_media(&src, app_dir, "cards/audio") {
            result.push(to_relative(&src, app_dir));
            continue;
        }

        let src_path = PathBuf::from(&src);
        if src_path.file_name().is_none() {
            continue;
        }
        let ext = src_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let uuid_name = if ext.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            format!("{}.{}", uuid::Uuid::new_v4(), ext)
        };
        let dest = dest_dir.join(&uuid_name);
        if std::fs::copy(&src_path, &dest).is_ok() {
            result.push(format!("cards/audio/{uuid_name}"));
        }
    }

    Ok(result)
}
