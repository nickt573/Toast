use crate::crud::create::{create_card_imported, create_deck};
use crate::crud::models::*;
use rusqlite::{Connection, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const FIELD_SEP: char = '\x1f';

/// Removes inline `on*="..."` event-handler attributes (onclick, onload, …)
/// from imported Anki HTML so it can't run scripts when rendered.
/// Byte scanner: `i` walks the input; on a match, `j` finds the end of the
/// attribute name and `k` the end of the quoted value, then `i` jumps past it.
fn strip_event_handlers(html: &str) -> String {
    let b = html.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let is_ws = matches!(b[i], b' ' | b'\t' | b'\n' | b'\r');
        if is_ws
            && i + 3 < b.len()
            && b[i + 1].to_ascii_lowercase() == b'o'
            && b[i + 2].to_ascii_lowercase() == b'n'
            && b[i + 3].is_ascii_alphabetic()
        {
            let mut j = i + 3;
            while j < b.len() && b[j].is_ascii_alphabetic() {
                j += 1;
            }
            let mut k = j;
            while k < b.len() && matches!(b[k], b' ' | b'\t') {
                k += 1;
            }
            if k < b.len() && b[k] == b'=' {
                k += 1;
                while k < b.len() && matches!(b[k], b' ' | b'\t') {
                    k += 1;
                }
                if k < b.len() && (b[k] == b'"' || b[k] == b'\'') {
                    let q = b[k];
                    k += 1;
                    while k < b.len() && b[k] != q {
                        k += 1;
                    }
                    if k < b.len() {
                        k += 1;
                    }
                    out.push(b[i]); // keep the leading whitespace
                    i = k;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| html.to_string())
}

fn strip_scripts(html: &str) -> String {
    let mut result = String::new();
    let mut rest = html;
    loop {
        let lower = rest.to_lowercase();
        match lower.find("<script") {
            None => {
                result.push_str(rest);
                break;
            }
            Some(start) => {
                result.push_str(&rest[..start]);
                match lower[start..].find("</script>") {
                    None => {
                        break;
                    }
                    Some(end) => {
                        rest = &rest[start + end + 9..];
                    }
                }
            }
        }
    }
    result
}

fn strip_cloze(html: &str) -> String {
    let mut result = String::new();
    let mut rest = html;
    loop {
        match rest.find("{{c") {
            None => {
                result.push_str(rest);
                break;
            }
            Some(start) => {
                result.push_str(&rest[..start]);
                match rest[start..].find("}}") {
                    None => {
                        result.push_str(&rest[start..]);
                        break;
                    }
                    Some(end) => {
                        result.push_str("___");
                        rest = &rest[start + end + 2..];
                    }
                }
            }
        }
    }
    result
}

/// Rewrites Anki media references to the copied files' absolute paths:
/// `src="file"` / `src='file'` values are replaced in place, and Anki's
/// `[sound:file]` markers become full `<audio>` tags.
/// NOTE: every branch must advance `pos` past what it consumed — a branch
/// that leaves `pos` at the match start would loop forever.
fn rewrite_media(html: &str, media_str_map: &HashMap<String, String>) -> String {
    let mut result = html.to_string();
    let mut pos = 0;

    while pos < result.len() {
        let src_dq = result[pos..]
            .find("src=\"")
            .map(|i| (i + pos, 5usize, '"', false));
        let src_sq = result[pos..]
            .find("src='")
            .map(|i| (i + pos, 5usize, '\'', false));
        let sound = result[pos..]
            .find("[sound:")
            .map(|i| (i + pos, 7usize, ']', true));

        let next = [src_dq, src_sq, sound]
            .into_iter()
            .flatten()
            .min_by_key(|(i, _, _, _)| *i);

        match next {
            None => break,
            Some((start, prefix_len, end_char, is_sound)) => {
                let after = &result[start + prefix_len..];
                if let Some(end) = after.find(end_char) {
                    let filename = after[..end].to_string();
                    if media_str_map.contains_key(&filename) {
                        if is_sound {
                            let dest = media_str_map.get(&filename).cloned().unwrap_or_default();
                            let audio_tag = format!("<audio controls src=\"{}\"></audio>", dest);
                            let tag_end = start + prefix_len + end + 1;
                            result.replace_range(start..tag_end, &audio_tag);
                            pos = start + audio_tag.len();
                        } else {
                            let dest = media_str_map[&filename].clone();
                            let from = start + prefix_len;
                            let to = from + end;
                            result.replace_range(from..to, &dest);
                            pos = from + dest.len();
                        }
                    } else {
                        pos = start + prefix_len + end + 1;
                    }
                } else {
                    pos = start + prefix_len;
                }
            }
        }
    }

    result
}

fn open_anki_db(tmp_dir: &tempfile::TempDir) -> Result<Connection> {
    let p21b = tmp_dir.path().join("collection.anki21b");
    let p21 = tmp_dir.path().join("collection.anki21");
    let p2 = tmp_dir.path().join("collection.anki2");
    let path = if p21b.exists() {
        p21b
    } else if p21.exists() {
        p21
    } else {
        p2
    };
    Connection::open(&path).map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))
}

const MAX_ZIP_ENTRIES: usize = 10_000;
const MAX_ZIP_FILE_BYTES: u64 = 100 * 1024 * 1024; // 100 MB per file
const MAX_ZIP_TOTAL_BYTES: u64 = 500 * 1024 * 1024; // 500 MB total

fn extract_zip(apkg_path: &str) -> Result<tempfile::TempDir> {
    let tmp_dir =
        tempfile::tempdir().map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
    let apkg_file = std::fs::File::open(apkg_path)
        .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
    let mut archive = zip::ZipArchive::new(apkg_file)
        .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;

    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "Archive has too many entries ({}); maximum is {}.",
            archive.len(),
            MAX_ZIP_ENTRIES
        )));
    }

    let mut total_bytes: u64 = 0;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;

        // Reject path traversal attempts
        let name = file.name().to_string();
        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
            continue;
        }

        let out_path = tmp_dir.path().join(&name);

        if file.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
        } else {
            if file.size() > MAX_ZIP_FILE_BYTES {
                continue; // Skip individual files that are too large
            }
            let mut out = std::fs::File::create(&out_path)
                .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
            let written = std::io::copy(&mut file, &mut out)
                .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
            total_bytes += written;
            if total_bytes > MAX_ZIP_TOTAL_BYTES {
                return Err(rusqlite::Error::InvalidParameterName(
                    "Archive total extraction size exceeds 500 MB limit.".to_string(),
                ));
            }
        }
    }
    Ok(tmp_dir)
}

fn read_deck_name(anki_conn: &Connection) -> String {
    let decks_json: String = anki_conn
        .query_row("SELECT decks FROM col LIMIT 1", [], |row| row.get(0))
        .unwrap_or_else(|_| "{}".to_string());

    let decks_val: Value =
        serde_json::from_str(&decks_json).unwrap_or(Value::Object(Default::default()));

    // id 1 is Anki's built-in "Default" deck; the imported deck is any other
    decks_val
        .as_object()
        .and_then(|m| m.values().find(|d| d["id"].as_i64().unwrap_or(0) != 1))
        .and_then(|d| d["name"].as_str())
        .unwrap_or("Imported Anki Deck")
        .to_string()
}

fn copy_media(tmp_dir: &tempfile::TempDir, app_dir: &Path) -> Result<HashMap<String, PathBuf>> {
    let media_manifest_path = tmp_dir.path().join("media");
    let media_json: HashMap<String, String> = if media_manifest_path.exists() {
        let content = std::fs::read_to_string(&media_manifest_path)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
        serde_json::from_str(&content)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?
    } else {
        HashMap::new()
    };

    let img_dest_dir = app_dir.join("cards").join("images");
    let aud_dest_dir = app_dir.join("cards").join("audio");
    std::fs::create_dir_all(&img_dest_dir)
        .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
    std::fs::create_dir_all(&aud_dest_dir)
        .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;

    let mut media_map: HashMap<String, PathBuf> = HashMap::new();
    for (num_key, filename) in &media_json {
        if filename.starts_with('_') {
            continue;
        }
        let src = tmp_dir.path().join(num_key);
        if !src.exists() {
            continue;
        }
        let ext = Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let is_audio = matches!(ext.as_str(), "mp3" | "wav" | "ogg" | "m4a" | "flac");

        // ── UUID rename to avoid collisions across duplicate imports ──
        let safe_name = if ext.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            format!("{}.{}", uuid::Uuid::new_v4(), ext)
        };
        let dest = if is_audio {
            aud_dest_dir.join(&safe_name)
        } else {
            img_dest_dir.join(&safe_name)
        };
        // ─────────────────────────────────────────────────────────────

        if std::fs::copy(&src, &dest).is_ok() {
            media_map.insert(filename.clone(), dest);
        }
    }
    Ok(media_map)
}

pub fn peek_anki_fields(apkg_path: &str) -> std::result::Result<Vec<String>, String> {
    let tmp_dir = extract_zip(apkg_path).map_err(|e| e.to_string())?;
    let anki_conn = open_anki_db(&tmp_dir).map_err(|e| e.to_string())?;

    let col_models_json: String = anki_conn
        .query_row("SELECT models FROM col LIMIT 1", [], |row| row.get(0))
        .or_else(|_| {
            anki_conn.query_row(
                "SELECT json_extract(json, '$.models') FROM col LIMIT 1",
                [],
                |row| row.get(0),
            )
        })
        .map_err(|e| e.to_string())?;

    let models: Value = serde_json::from_str(&col_models_json).map_err(|e| e.to_string())?;

    let fields: Vec<String> = if let Value::Object(ref m) = models {
        if let Some((_, model)) = m.iter().next() {
            if let Some(arr) = model["flds"].as_array() {
                arr.iter()
                    .filter_map(|f| f["name"].as_str().map(|s| s.to_string()))
                    .collect()
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    if fields.is_empty() {
        Ok(vec!["Field 1".to_string(), "Field 2".to_string()])
    } else {
        Ok(fields)
    }
}

pub fn import_anki_deck(
    apkg_path: &str,
    app_dir: &Path,
    conn: &mut Connection,
    front_field_indices: Vec<usize>,
    back_field_indices: Vec<usize>,
    support_field_indices: Vec<usize>,
    create_flipped: bool,
    is_searchable: bool,
) -> Result<(i64, usize)> {
    let tmp_dir = extract_zip(apkg_path)?;
    let anki_conn = open_anki_db(&tmp_dir)?;
    let media_map = copy_media(&tmp_dir, app_dir)?;
    let deck_name = read_deck_name(&anki_conn);

    struct NoteRow {
        flds: String,
    }

    let notes: Vec<NoteRow> = {
        let mut stmt = anki_conn.prepare("SELECT flds FROM notes")?;
        let rows = stmt
            .query_map([], |row| Ok(NoteRow { flds: row.get(0)? }))?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };

    let media_str_map: HashMap<String, String> = media_map
        .iter()
        .map(|(k, v)| (k.clone(), v.to_string_lossy().to_string()))
        .collect();

    let tx = conn.transaction()?;
    let new_deck = create_deck(deck_name, &*tx)?;

    let mut card_count = 0;

    for note in notes {
        let raw_fields: Vec<&str> = note.flds.split(FIELD_SEP).collect();
        if raw_fields.is_empty() {
            continue;
        }

        let max_idx = raw_fields.len().saturating_sub(1);

        // Empty fields are skipped so they don't leave stray <hr/> dividers
        let front_cloze = front_field_indices
            .iter()
            .filter(|&&i| i <= max_idx)
            .map(|&i| strip_cloze(raw_fields[i]))
            .filter(|f| !f.trim().is_empty())
            .collect::<Vec<_>>()
            .join("<hr/>");
        let back_cloze = back_field_indices
            .iter()
            .filter(|&&i| i <= max_idx)
            .map(|&i| strip_cloze(raw_fields[i]))
            .filter(|f| !f.trim().is_empty())
            .collect::<Vec<_>>()
            .join("<hr/>");
        let support_cloze = support_field_indices
            .iter()
            .filter(|&&i| i <= max_idx)
            .map(|&i| strip_cloze(raw_fields[i]))
            .filter(|f| !f.trim().is_empty())
            .collect::<Vec<_>>()
            .join("<hr/>");

        if front_cloze.trim().is_empty() && back_cloze.trim().is_empty() {
            continue;
        }

        let front_html = strip_event_handlers(&strip_scripts(&front_cloze));
        let back_html = strip_event_handlers(&strip_scripts(&back_cloze));
        let support_html = strip_event_handlers(&strip_scripts(&support_cloze));

        let front_clean = rewrite_media(&front_html, &media_str_map);
        let back_clean = rewrite_media(&back_html, &media_str_map);
        // Support fields stay out of front/back so they never pollute
        // similar-card matching; stored read-only in imported_support.
        let support_clean = if support_html.trim().is_empty() {
            None
        } else {
            Some(rewrite_media(&support_html, &media_str_map))
        };

        let new_card = NewCard {
            group_id: new_deck.id,
            front: front_clean.clone(),
            back: back_clean.clone(),
            is_searchable: is_searchable,
            is_uploaded: true,
            support: None,
            imported_support: support_clean.clone(),
            front_image: None,
            back_image: None,
            front_audio: None,
            back_audio: None,
        };

        create_card_imported(new_card, &tx)?;
        card_count += 1;

        if create_flipped {
            let flipped = NewCard {
                group_id: new_deck.id,
                front: back_clean,
                back: front_clean,
                is_searchable: is_searchable,
                is_uploaded: true,
                support: None,
                // Support is side-agnostic (always shown after flip), so the
                // flipped copy carries the same imported support.
                imported_support: support_clean,
                front_image: None,
                back_image: None,
                front_audio: None,
                back_audio: None,
            };
            create_card_imported(flipped, &tx)?;
            card_count += 1;
        }
    }

    tx.commit()?;

    Ok((new_deck.id, card_count))
}
