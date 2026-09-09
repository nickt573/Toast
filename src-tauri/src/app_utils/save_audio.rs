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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_and_empty_yield_none() {
        let app = tempfile::tempdir().unwrap();
        assert_eq!(save_card_audio_file(None, app.path()).unwrap(), None);
        assert_eq!(
            save_card_audio_file(Some(String::new()), app.path()).unwrap(),
            None
        );
    }

    #[test]
    fn copies_an_external_file_into_cards_audio() {
        let app = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let src_file = src.path().join("clip.mp3");
        std::fs::write(&src_file, b"id3fake").unwrap();

        let stored = save_card_audio_file(Some(src_file.to_string_lossy().into_owned()), app.path())
            .unwrap()
            .expect("a stored path");

        assert!(stored.starts_with("cards/audio/"), "relative under the audio dir: {stored}");
        assert!(stored.ends_with(".mp3"), "keeps the extension: {stored}");
        assert_eq!(
            std::fs::read(app.path().join(&stored)).unwrap(),
            b"id3fake",
            "bytes copied verbatim"
        );
    }

    #[test]
    fn an_already_stored_path_is_not_copied_again() {
        let app = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let src_file = src.path().join("clip.wav");
        std::fs::write(&src_file, b"riff").unwrap();

        let stored = save_card_audio_file(Some(src_file.to_string_lossy().into_owned()), app.path())
            .unwrap()
            .unwrap();
        let count_before = std::fs::read_dir(app.path().join("cards/audio")).unwrap().count();

        // Saving the stored relative path again returns it unchanged and writes no new file
        let again = save_card_audio_file(Some(stored.clone()), app.path()).unwrap().unwrap();
        assert_eq!(again, stored);
        let count_after = std::fs::read_dir(app.path().join("cards/audio")).unwrap().count();
        assert_eq!(count_before, count_after, "no duplicate copy made");
    }
}
