use crate::app_utils::paths::{is_stored_media, to_relative};
use crate::app_utils::save_img::save_page_image;
use rusqlite::Result;
use serde_json::Value;
use std::path::Path;

pub fn rewrite_images_in_content(content: &str, app_dir: &Path) -> Result<String> {
    let mut json: Value =
        serde_json::from_str(content).unwrap_or(Value::Object(serde_json::Map::new()));

    rewrite_image_nodes(&mut json, app_dir)?;

    Ok(json.to_string())
}

pub fn rewrite_image_nodes(node: &mut Value, app_dir: &Path) -> Result<()> {
    if let Some(obj) = node.as_object_mut() {
        if obj.get("type").and_then(|t| t.as_str()) == Some("image") {
            if let Some(attrs) = obj.get_mut("attrs") {
                if let Some(src) = attrs
                    .get("src")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string())
                {
                    if src.starts_with("http") || src.starts_with("asset:") || src.starts_with("data:") {
                        // Not a local file — leave alone
                    } else if is_stored_media(&src, app_dir, "pages/images") {
                        // Already stored: normalize in place (legacy absolute → relative)
                        let rel = to_relative(&src, app_dir);
                        attrs["src"] = Value::String(rel.clone());
                        attrs["rawPath"] = Value::String(rel);
                    } else if let Some(new_path) = save_page_image(Some(src), app_dir)? {
                        // rawPath must track the stored copy too — display prefers
                        // it, and it would otherwise keep the picked file's path
                        attrs["src"] = Value::String(new_path.clone());
                        attrs["rawPath"] = Value::String(new_path);
                    }
                }
            }
        }

        if let Some(children) = obj.get_mut("content") {
            if let Some(arr) = children.as_array_mut() {
                for child in arr.iter_mut() {
                    rewrite_image_nodes(child, app_dir)?;
                }
            }
        }
    }

    Ok(())
}

pub fn extract_image_paths(content: &str) -> Vec<String> {
    let mut paths = Vec::new();
    if let Ok(json) = serde_json::from_str::<Value>(content) {
        collect_image_paths(&json, &mut paths);
    }
    paths
}

pub fn collect_image_paths(node: &Value, paths: &mut Vec<String>) {
    if let Some(obj) = node.as_object() {
        if obj.get("type").and_then(|t| t.as_str()) == Some("image") {
            if let Some(src) = obj
                .get("attrs")
                .and_then(|a| a.get("src"))
                .and_then(|s| s.as_str())
            {
                paths.push(src.to_string());
            }
        }
        if let Some(content) = obj.get("content").and_then(|c| c.as_array()) {
            for child in content {
                collect_image_paths(child, paths);
            }
        }
    }
}

/// Paths present in old_content but not new_content. Both sides are compared
/// in relative form so a legacy absolute path never diffs against its own
/// relative equivalent (which would delete a still-referenced file).
pub fn removed_image_paths(old_content: &str, new_content: &str, app_dir: &Path) -> Vec<String> {
    let old_paths: Vec<String> = extract_image_paths(old_content)
        .iter()
        .map(|p| to_relative(p, app_dir))
        .collect();
    let new_paths: Vec<String> = extract_image_paths(new_content)
        .iter()
        .map(|p| to_relative(p, app_dir))
        .collect();
    old_paths
        .into_iter()
        .filter(|p| !new_paths.contains(p))
        .collect()
}

/// Collects local `src` attribute values from card HTML for file cleanup.
/// NOTE: it matches every `src=` regardless of tag, so <audio> sources are
/// collected here too — callers only use the paths for deletion, where that
/// overlap is harmless.
pub fn extract_image_paths_from_html(html: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut rest = html;
    loop {
        let dq = rest.find("src=\"").map(|i| (i, 5usize, '"'));
        let sq = rest.find("src='").map(|i| (i, 5usize, '\''));
        let next = match (dq, sq) {
            (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        match next {
            None => break,
            Some((start, prefix_len, end_char)) => {
                let after = &rest[start + prefix_len..];
                if let Some(end) = after.find(end_char) {
                    let src = &after[..end];
                    if !src.starts_with("http") && !src.starts_with("asset://") && !src.is_empty() {
                        paths.push(src.to_string());
                    }
                    rest = &after[end + 1..];
                } else {
                    break;
                }
            }
        }
    }
    paths
}
