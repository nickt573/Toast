use crate::app_utils::manage_img::extract_image_paths_from_html;
use crate::app_utils::paths::{resolve_media_path, to_relative, MEDIA_SUBDIRS};
use rusqlite::{Error, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

/// Copies one referenced media file to a fresh uuid-named file in the same subdir, with a
/// cache so a file referenced twice makes one copy, and a missing source passes through
pub fn copy_media_file(
    stored: &str,
    app_dir: &Path,
    cache: &mut HashMap<String, String>,
) -> Result<String> {
    if stored.is_empty() {
        return Ok(stored.to_string());
    }
    let rel = to_relative(stored, app_dir);
    if let Some(existing) = cache.get(&rel) {
        return Ok(existing.clone());
    }
    // Only remap paths living in one of our media subdirs, a url or a stray value passes
    // straight through
    let Some(subdir) = MEDIA_SUBDIRS
        .iter()
        .find(|d| rel.starts_with(&format!("{d}/")))
    else {
        return Ok(stored.to_string());
    };

    let src = resolve_media_path(app_dir, &rel);
    if !src.is_file() {
        return Ok(stored.to_string());
    }

    let ext = Path::new(&rel)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let new_name = if ext.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        format!("{}.{}", Uuid::new_v4(), ext)
    };
    let new_rel = format!("{subdir}/{new_name}");

    let dest = app_dir.join(subdir).join(&new_name);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::InvalidParameterName(e.to_string()))?;
    }
    std::fs::copy(&src, &dest).map_err(|e| Error::InvalidParameterName(e.to_string()))?;

    cache.insert(rel, new_rel.clone());
    Ok(new_rel)
}

/// Copies an optional media column, leaving None and empty strings untouched
pub fn copy_media_opt(
    stored: &Option<String>,
    app_dir: &Path,
    cache: &mut HashMap<String, String>,
) -> Result<Option<String>> {
    match stored {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(Some(s.clone())),
        Some(s) => Ok(Some(copy_media_file(s, app_dir, cache)?)),
    }
}

/// Copies every media file referenced by imported Anki HTML and rewrites the src values to
/// the new copies, images and audio alike since the extractor matches every tag
pub fn copy_html_media(
    html: &Option<String>,
    app_dir: &Path,
    cache: &mut HashMap<String, String>,
) -> Result<Option<String>> {
    let Some(html) = html.as_ref() else {
        return Ok(None);
    };
    let mut out = html.clone();
    for src in extract_image_paths_from_html(html) {
        let new = copy_media_file(&src, app_dir, cache)?;
        if new != src {
            out = out.replace(&src, &new);
        }
    }
    Ok(Some(out))
}

/// Copies every image referenced in a page's editor JSON and rewrites both src and rawPath
/// to the new copies
pub fn copy_page_content_media(
    content: &str,
    app_dir: &Path,
    cache: &mut HashMap<String, String>,
) -> Result<String> {
    let Ok(mut json) = serde_json::from_str::<Value>(content) else {
        return Ok(content.to_string());
    };
    remap_image_nodes(&mut json, app_dir, cache)?;
    Ok(json.to_string())
}

/// Deletes the copies made so far to unwind a failed duplicate, since the cache values are
/// exactly the new files this operation created
pub fn remove_copied_files(cache: &HashMap<String, String>, app_dir: &Path) {
    for new_rel in cache.values() {
        let _ = std::fs::remove_file(app_dir.join(new_rel));
    }
}

fn remap_image_nodes(
    node: &mut Value,
    app_dir: &Path,
    cache: &mut HashMap<String, String>,
) -> Result<()> {
    if let Some(obj) = node.as_object_mut() {
        if obj.get("type").and_then(|t| t.as_str()) == Some("image") {
            if let Some(attrs) = obj.get_mut("attrs") {
                if let Some(src) = attrs.get("src").and_then(|s| s.as_str()).map(String::from) {
                    let new = copy_media_file(&src, app_dir, cache)?;
                    if new != src {
                        attrs["src"] = Value::String(new.clone());
                        attrs["rawPath"] = Value::String(new);
                    }
                }
            }
        }
        if let Some(children) = obj.get_mut("content").and_then(|c| c.as_array_mut()) {
            for child in children.iter_mut() {
                remap_image_nodes(child, app_dir, cache)?;
            }
        }
    }
    Ok(())
}
