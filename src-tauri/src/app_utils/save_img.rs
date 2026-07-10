use crate::app_utils::paths::{is_stored_media, to_relative};
use rusqlite::{Error, Result};
use std::{fs, path::Path, path::PathBuf};
use uuid::Uuid;

pub fn save_card_image(src_path: Option<String>, app_dir: &Path) -> Result<Option<String>> {
    let src = match src_path {
        None => return Ok(None),
        Some(p) if p.is_empty() => return Ok(None),
        Some(p) => p,
    };

    let dest_dir = app_dir.join("cards").join("images");
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;

    // Already stored (relative or a legacy absolute form) — keep, normalized
    if is_stored_media(&src, app_dir, "cards/images") {
        return Ok(Some(to_relative(&src, app_dir)));
    }

    let src_path = PathBuf::from(&src);
    let ext = src_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg")
        .to_lowercase();
    let uuid_name = format!("{}.{}", uuid::Uuid::new_v4(), ext);
    let dest = dest_dir.join(&uuid_name);
    std::fs::copy(&src_path, &dest)
        .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;

    Ok(Some(format!("cards/images/{uuid_name}")))
}

pub fn save_page_image(image_path: Option<String>, app_dir: &Path) -> Result<Option<String>> {
    let Some(image_path) = image_path else {
        return Ok(None);
    };

    // Already stored (relative or a legacy absolute form) — keep, normalized
    if is_stored_media(&image_path, app_dir, "pages/images") {
        return Ok(Some(to_relative(&image_path, app_dir)));
    }

    let src = Path::new(&image_path);

    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("png");

    let images_dir = app_dir.join("pages").join("images");

    fs::create_dir_all(&images_dir).map_err(|e| Error::InvalidParameterName(e.to_string()))?;

    let filename = format!("{}.{}", Uuid::new_v4(), ext);
    let dest = images_dir.join(&filename);

    fs::copy(src, &dest).map_err(|e| Error::InvalidParameterName(e.to_string()))?;

    Ok(Some(format!("pages/images/{filename}")))
}
