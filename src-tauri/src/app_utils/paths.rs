use std::path::{Path, PathBuf};

/// Media directories under the app data dir. Stored paths are relative to
/// app_dir and always use forward slashes, e.g. "cards/images/<uuid>.png".
pub const MEDIA_SUBDIRS: [&str; 4] = ["cards/images", "cards/audio", "pages/images", "pages/audio"];

fn is_url(path: &str) -> bool {
    path.starts_with("http")
        || path.starts_with("asset:")
        || path.starts_with("data:")
        || path.starts_with("blob:")
}

/// Canonicalizes a stored media reference to app_dir-relative form.
/// Absolute paths under the current app_dir are stripped to their media subpath.
/// Absolute paths under a stale root (renamed username, app dir copied from another machine)
/// are recovered by locating the media subdir marker. URLs and non-media paths are unchanged.
pub fn to_relative(path: &str, app_dir: &Path) -> String {
    if path.is_empty() || is_url(path) {
        return path.to_string();
    }

    // Normalize separators so Windows-stored paths relativize too
    let normalized = path.replace('\\', "/");

    let app_dir_str = app_dir.to_string_lossy().replace('\\', "/");
    let app_prefix = format!("{}/", app_dir_str.trim_end_matches('/'));
    if let Some(rest) = normalized.strip_prefix(&app_prefix) {
        for subdir in MEDIA_SUBDIRS {
            if rest.starts_with(&format!("{subdir}/")) {
                return rest.to_string();
            }
        }
    }

    // Stale root: keep everything from the media subdir marker onward
    for subdir in MEDIA_SUBDIRS {
        let marker = format!("/{subdir}/");
        if let Some(idx) = normalized.find(&marker) {
            return normalized[idx + 1..].to_string();
        }
    }

    path.to_string()
}

/// Resolves a stored path for filesystem access: rooted/absolute paths pass
/// through (has_root so Unix-absolute strings aren't mis-joined on Windows),
/// relative paths are joined to app_dir.
pub fn resolve_media_path(app_dir: &Path, stored: &str) -> PathBuf {
    let p = Path::new(stored);
    if p.has_root() || p.is_absolute() {
        p.to_path_buf()
    } else {
        app_dir.join(stored)
    }
}

/// True if `path` already refers to a saved file in `subdir` ("cards/images",
/// …): relative under it, or absolute under it in the current or a stale
/// app_dir. Such paths must not be copied again on save.
pub fn is_stored_media(path: &str, app_dir: &Path, subdir: &str) -> bool {
    if path.is_empty() || is_url(path) {
        return false;
    }
    to_relative(path, app_dir).starts_with(&format!("{subdir}/"))
}

/// Rewrites src="…" / src='…' attribute values in card HTML to relative form.
/// Returns None when nothing changed. Scans like extract_image_paths_from_html
/// so exactly the same references are matched.
pub fn relativize_html_media(html: &str, app_dir: &Path) -> Option<String> {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut changed = false;

    loop {
        let dq = rest.find("src=\"").map(|i| (i, '"'));
        let sq = rest.find("src='").map(|i| (i, '\''));
        let next = match (dq, sq) {
            (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        match next {
            None => break,
            Some((start, quote)) => {
                let value_start = start + 5;
                out.push_str(&rest[..value_start]);
                let after = &rest[value_start..];
                match after.find(quote) {
                    None => {
                        rest = after;
                        break;
                    }
                    Some(end) => {
                        let src = &after[..end];
                        let rel = to_relative(src, app_dir);
                        if rel != src {
                            changed = true;
                        }
                        out.push_str(&rel);
                        rest = &after[end..];
                    }
                }
            }
        }
    }
    out.push_str(rest);

    changed.then_some(out)
}

/// Relativizes attrs.src of every TipTap image node and syncs attrs.rawPath to the stored copy.
/// rawPath historically kept pointing at the originally picked file, not the app-dir copy.
/// Returns true if anything changed.
pub fn relativize_image_nodes(node: &mut serde_json::Value, app_dir: &Path) -> bool {
    use serde_json::Value;

    let mut changed = false;
    let Some(obj) = node.as_object_mut() else {
        return false;
    };

    if obj.get("type").and_then(|t| t.as_str()) == Some("image") {
        if let Some(attrs) = obj.get_mut("attrs").and_then(|a| a.as_object_mut()) {
            if let Some(src) = attrs.get("src").and_then(|s| s.as_str()).map(str::to_string) {
                let new_src = to_relative(&src, app_dir);
                if new_src != src {
                    attrs.insert("src".into(), Value::String(new_src.clone()));
                    changed = true;
                }
                if let Some(raw) = attrs.get("rawPath").and_then(|r| r.as_str()).map(str::to_string) {
                    let is_media = MEDIA_SUBDIRS
                        .iter()
                        .any(|s| new_src.starts_with(&format!("{s}/")));
                    let new_raw = if is_media { new_src } else { to_relative(&raw, app_dir) };
                    if new_raw != raw {
                        attrs.insert("rawPath".into(), Value::String(new_raw));
                        changed = true;
                    }
                }
            }
        }
    }

    if let Some(children) = obj.get_mut("content").and_then(|c| c.as_array_mut()) {
        for child in children {
            changed |= relativize_image_nodes(child, app_dir);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_dir() -> PathBuf {
        PathBuf::from("/home/alice/.local/share/com.toast.app")
    }

    #[test]
    fn strips_current_app_dir_prefix() {
        assert_eq!(
            to_relative(
                "/home/alice/.local/share/com.toast.app/cards/images/a.png",
                &app_dir()
            ),
            "cards/images/a.png"
        );
    }

    #[test]
    fn strips_stale_root_via_marker() {
        assert_eq!(
            to_relative(
                "/home/renamed/.local/share/com.toast.app/cards/audio/b.mp3",
                &app_dir()
            ),
            "cards/audio/b.mp3"
        );
    }

    #[test]
    fn strips_macos_style_path() {
        // Databases created on macOS store this form (note the space in
        // "Application Support") and must port cleanly to other machines
        assert_eq!(
            to_relative(
                "/Users/bob/Library/Application Support/com.toast.app/cards/audio/e.mp3",
                &app_dir()
            ),
            "cards/audio/e.mp3"
        );
        assert_eq!(
            to_relative(
                "/Users/bob/Library/Application Support/com.toast.app/pages/audio/f.mp4",
                &PathBuf::from("/Users/bob/Library/Application Support/com.toast.app")
            ),
            "pages/audio/f.mp4"
        );
    }

    #[test]
    fn strips_windows_style_path() {
        assert_eq!(
            to_relative(
                r"C:\Users\alice\AppData\Roaming\com.toast.app\pages\images\c.png",
                &app_dir()
            ),
            "pages/images/c.png"
        );
    }

    #[test]
    fn leaves_relative_paths_unchanged() {
        assert_eq!(
            to_relative("pages/audio/d.mp4", &app_dir()),
            "pages/audio/d.mp4"
        );
    }

    #[test]
    fn leaves_urls_unchanged() {
        for url in [
            "http://example.com/x.png",
            "https://example.com/x.png",
            "asset://localhost/x",
            "data:image/png;base64,AAAA",
            "blob:null/abc",
        ] {
            assert_eq!(to_relative(url, &app_dir()), url);
        }
    }

    #[test]
    fn leaves_non_media_absolute_unchanged() {
        assert_eq!(
            to_relative("/home/alice/Pictures/photo.png", &app_dir()),
            "/home/alice/Pictures/photo.png"
        );
    }

    #[test]
    fn resolve_joins_relative_and_passes_absolute() {
        assert_eq!(
            resolve_media_path(&app_dir(), "cards/images/a.png"),
            app_dir().join("cards/images/a.png")
        );
        assert_eq!(
            resolve_media_path(&app_dir(), "/tmp/x.png"),
            PathBuf::from("/tmp/x.png")
        );
    }

    #[test]
    fn is_stored_media_matches_forms() {
        assert!(is_stored_media("cards/images/a.png", &app_dir(), "cards/images"));
        assert!(is_stored_media(
            "/home/alice/.local/share/com.toast.app/cards/images/a.png",
            &app_dir(),
            "cards/images"
        ));
        assert!(is_stored_media(
            "/home/other/.local/share/com.toast.app/cards/images/a.png",
            &app_dir(),
            "cards/images"
        ));
        assert!(!is_stored_media("/home/alice/Pictures/a.png", &app_dir(), "cards/images"));
        assert!(!is_stored_media("cards/audio/a.mp3", &app_dir(), "cards/images"));
        assert!(!is_stored_media("https://x.com/cards/images/a.png", &app_dir(), "cards/images"));
    }

    #[test]
    fn relativizes_html_srcs() {
        let html = r#"<img src="/home/alice/.local/share/com.toast.app/cards/images/a.png"><audio controls src='/home/old/.local/share/com.toast.app/cards/audio/b.mp3'></audio><img src="https://x.com/k.png">"#;
        let out = relativize_html_media(html, &app_dir()).unwrap();
        assert_eq!(
            out,
            r#"<img src="cards/images/a.png"><audio controls src='cards/audio/b.mp3'></audio><img src="https://x.com/k.png">"#
        );
    }

    #[test]
    fn relativizes_image_nodes_and_syncs_raw_path() {
        let mut doc: serde_json::Value = serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "image",
                "attrs": {
                    "src": "/home/alice/.local/share/com.toast.app/pages/images/a.png",
                    "rawPath": "/home/alice/Pictures/original.png"
                }
            }]
        });
        assert!(relativize_image_nodes(&mut doc, &app_dir()));
        let attrs = &doc["content"][0]["attrs"];
        assert_eq!(attrs["src"], "pages/images/a.png");
        assert_eq!(attrs["rawPath"], "pages/images/a.png");
        // second pass is a no-op
        assert!(!relativize_image_nodes(&mut doc, &app_dir()));
    }

    #[test]
    fn relativize_html_returns_none_when_unchanged() {
        let html = r#"<img src="cards/images/a.png"><img src="https://x.com/k.png">"#;
        assert!(relativize_html_media(html, &app_dir()).is_none());
    }
}
