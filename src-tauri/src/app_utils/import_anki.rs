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

/// Copies one extracted media file into the cards dir under a fresh UUID name.
fn copy_media_file(src: &Path, filename: &str, app_dir: &Path) -> Option<String> {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let is_audio = matches!(ext.as_str(), "mp3" | "wav" | "ogg" | "opus" | "m4a" | "mp4" | "flac" | "webm");

    let subdir = if is_audio { "cards/audio" } else { "cards/images" };
    let dest_dir = app_dir.join(subdir);
    if std::fs::create_dir_all(&dest_dir).is_err() {
        return None;
    }

    let safe_name = if ext.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        format!("{}.{}", uuid::Uuid::new_v4(), ext)
    };
    let dest = dest_dir.join(&safe_name);
    if std::fs::copy(src, &dest).is_ok() {
        Some(format!("{subdir}/{safe_name}"))
    } else {
        None
    }
}

/// Rewrites `src` values in place and turns `[sound:file]` into `<audio>` tags.
/// Files are copied on first reference and cached in `copies`, which must be
/// scoped to one card so no two cards ever share a file on disk.
/// NOTE: every branch must advance `pos` past what it consumed, or it loops forever.
fn rewrite_media(
    html: &str,
    sources: &HashMap<String, PathBuf>,
    app_dir: &Path,
    copies: &mut HashMap<String, String>,
) -> String {
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
                    let dest = match copies.get(&filename) {
                        Some(d) => Some(d.clone()),
                        None => sources.get(&filename).and_then(|src| {
                            let copied = copy_media_file(src, &filename, app_dir);
                            if let Some(ref d) = copied {
                                copies.insert(filename.clone(), d.clone());
                            }
                            copied
                        }),
                    };
                    if let Some(dest) = dest {
                        if is_sound {
                            let audio_tag = format!("<audio controls src=\"{}\"></audio>", dest);
                            let tag_end = start + prefix_len + end + 1;
                            result.replace_range(start..tag_end, &audio_tag);
                            pos = start + audio_tag.len();
                        } else {
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

/// Maps Anki media filenames to their extracted temp-file paths.
fn media_sources(tmp_dir: &tempfile::TempDir) -> Result<HashMap<String, PathBuf>> {
    let media_manifest_path = tmp_dir.path().join("media");
    let media_json: HashMap<String, String> = if media_manifest_path.exists() {
        let content = std::fs::read_to_string(&media_manifest_path)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
        serde_json::from_str(&content)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?
    } else {
        HashMap::new()
    };

    let mut sources: HashMap<String, PathBuf> = HashMap::new();
    for (num_key, filename) in &media_json {
        if filename.starts_with('_') {
            continue;
        }
        let src = tmp_dir.path().join(num_key);
        if !src.exists() {
            continue;
        }
        sources.insert(filename.clone(), src);
    }
    Ok(sources)
}

/// A selectable field on the mapping screen. Field names routinely misdescribe
/// their contents, so samples are what you actually map against. `note_count` is
/// how many notes have something in it (a type can declare a field its notes never fill).
#[derive(serde::Serialize)]
pub struct AnkiField {
    pub name: String,
    pub note_count: i64,
    pub samples: Vec<AnkiSample>,
}

/// One note's value for a field. `media` names the kinds of media it carries
/// ("audio", "image"), which the UI must describe rather than print (the field
/// holds a sound file, it doesn't hold the word "audio").
#[derive(serde::Serialize, PartialEq)]
pub struct AnkiSample {
    pub text: String,
    pub media: Vec<String>,
}

impl AnkiSample {
    fn is_empty(&self) -> bool {
        self.text.is_empty() && self.media.is_empty()
    }
}

const SAMPLES_PER_FIELD: usize = 8;
const SAMPLE_MAX_CHARS: usize = 90;

/// Flattens a raw Anki field to a short line of plain text plus the media it holds.
fn preview_sample(raw: &str) -> AnkiSample {
    let mut media: Vec<String> = Vec::new();

    let mut text = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("[sound:") {
        text.push_str(&rest[..start]);
        match rest[start..].find(']') {
            Some(end) => {
                if !media.iter().any(|m| m == "audio") {
                    media.push("audio".to_string());
                }
                rest = &rest[start + end + 1..];
            }
            None => break,
        }
    }
    text.push_str(rest);

    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    let mut tag = String::new();
    for c in text.chars() {
        match c {
            '<' => {
                depth += 1;
                tag.clear();
            }
            '>' if depth > 0 => {
                depth -= 1;
                if tag.starts_with("img") && !media.iter().any(|m| m == "image") {
                    media.push("image".to_string());
                }
            }
            _ if depth > 0 => tag.push(c.to_ascii_lowercase()),
            _ => out.push(c),
        }
    }

    let out = out.replace("&nbsp;", " ").replace("&amp;", "&");
    let mut text = out.split_whitespace().collect::<Vec<_>>().join(" ");

    if text.chars().count() > SAMPLE_MAX_CHARS {
        text = text.chars().take(SAMPLE_MAX_CHARS).collect::<String>() + "…";
    }

    AnkiSample { text, media }
}

#[derive(Default)]
struct FieldScan {
    filled: i64,
    samples: Vec<AnkiSample>,
}

/// Deterministic xorshift, enough to spread samples across the deck without pulling in an RNG dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Walks every note once, tallying how many actually fill each field and reservoir-
/// sampling a few values to preview. Sampling the whole deck rather than a window of
/// it matters twice over: a field's content can start well past the first notes, and
/// the first notes of a deck are rarely representative of it.
fn scan_notes(
    anki_conn: &Connection,
    models: &HashMap<i64, Vec<String>>,
) -> std::result::Result<(i64, HashMap<i64, i64>, HashMap<String, FieldScan>), String> {
    let mut stmt = anki_conn
        .prepare("SELECT mid, flds FROM notes")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;

    let mut total: i64 = 0;
    let mut per_model: HashMap<i64, i64> = HashMap::new();
    let mut scans: HashMap<String, FieldScan> = HashMap::new();
    let mut rng = Rng(0x9E3779B97F4A7C15);

    for (mid, flds) in rows.filter_map(|r| r.ok()) {
        total += 1;
        *per_model.entry(mid).or_insert(0) += 1;

        let Some(layout) = models.get(&mid) else { continue };
        let values: Vec<&str> = flds.split(FIELD_SEP).collect();

        for (ord, name) in layout.iter().enumerate() {
            let Some(value) = values.get(ord) else { continue };
            let sample = preview_sample(value);
            if sample.is_empty() {
                continue;
            }

            let scan = scans.entry(name.clone()).or_default();
            scan.filled += 1;
            if scan.samples.contains(&sample) {
                continue;
            }

            if scan.samples.len() < SAMPLES_PER_FIELD {
                scan.samples.push(sample);
            } else {
                // Reservoir: replace with probability SAMPLES_PER_FIELD/filled, so
                // every distinct value in the deck has an even chance of showing.
                let roll = (rng.next() % scan.filled.max(1) as u64) as usize;
                if roll < SAMPLES_PER_FIELD {
                    scan.samples[roll] = sample;
                }
            }
        }
    }

    Ok((total, per_model, scans))
}

#[derive(serde::Serialize)]
pub struct AnkiPeek {
    pub total_notes: i64,
    pub fields: Vec<AnkiField>,
}

/// Field names of every note type, keyed by note-type id and ordered by `ord`.
fn read_models(anki_conn: &Connection) -> std::result::Result<HashMap<i64, Vec<String>>, String> {
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

    let mut out: HashMap<i64, Vec<String>> = HashMap::new();
    if let Value::Object(ref m) = models {
        for (mid, model) in m {
            let Ok(mid) = mid.parse::<i64>() else { continue };
            let Some(arr) = model["flds"].as_array() else { continue };
            let mut flds: Vec<(i64, String)> = arr
                .iter()
                .filter_map(|f| {
                    let name = f["name"].as_str()?.to_string();
                    Some((f["ord"].as_i64().unwrap_or(0), name))
                })
                .collect();
            flds.sort_by_key(|(ord, _)| *ord);
            out.insert(mid, flds.into_iter().map(|(_, name)| name).collect());
        }
    }
    Ok(out)
}

/// The union of field names across every note type that has notes, ordered by the
/// note types that use them most. Mapping by name rather than position is what
/// keeps a multi-note-type deck from scrambling, since the same ordinal means
/// different things in each.
pub fn peek_anki_fields(apkg_path: &str) -> std::result::Result<AnkiPeek, String> {
    let tmp_dir = extract_zip(apkg_path).map_err(|e| e.to_string())?;
    let anki_conn = open_anki_db(&tmp_dir).map_err(|e| e.to_string())?;

    let models = read_models(&anki_conn)?;
    let (total_notes, per_model, mut scans) = scan_notes(&anki_conn, &models)?;

    // Biggest note type first so its fields lead the list.
    let mut used: Vec<(&i64, &i64)> = per_model.iter().filter(|(_, &c)| c > 0).collect();
    used.sort_by(|a, b| b.1.cmp(a.1));

    let mut order: Vec<String> = Vec::new();
    for (mid, _) in used {
        let Some(fields) = models.get(mid) else { continue };
        for name in fields {
            if !order.contains(name) {
                order.push(name.clone());
            }
        }
    }

    if order.is_empty() {
        return Ok(AnkiPeek {
            total_notes,
            fields: vec![
                AnkiField { name: "Field 1".to_string(), note_count: 0, samples: vec![] },
                AnkiField { name: "Field 2".to_string(), note_count: 0, samples: vec![] },
            ],
        });
    }

    let fields = order
        .into_iter()
        .map(|name| {
            let scan = scans.remove(&name).unwrap_or_default();
            AnkiField { name, note_count: scan.filled, samples: scan.samples }
        })
        .collect();

    Ok(AnkiPeek { total_notes, fields })
}

/// Joins the selected field names for one note, resolving each name against that
/// note's own note type. Names the note type doesn't have, and empty values, are
/// skipped so they don't leave stray <hr/> dividers.
fn collect_fields(selected: &[String], layout: &[String], values: &[&str]) -> String {
    selected
        .iter()
        .filter_map(|name| {
            let ord = layout.iter().position(|f| f == name)?;
            let value = values.get(ord)?;
            let stripped = strip_cloze(value);
            (!stripped.trim().is_empty()).then_some(stripped)
        })
        .collect::<Vec<_>>()
        .join("<hr/>")
}

pub fn import_anki_deck(
    apkg_path: &str,
    app_dir: &Path,
    conn: &mut Connection,
    front_fields: Vec<String>,
    back_fields: Vec<String>,
    support_fields: Vec<String>,
    create_flipped: bool,
    is_searchable: bool,
) -> Result<(i64, usize)> {
    let tmp_dir = extract_zip(apkg_path)?;
    let anki_conn = open_anki_db(&tmp_dir)?;
    let sources = media_sources(&tmp_dir)?;
    let deck_name = read_deck_name(&anki_conn);
    let models = read_models(&anki_conn)
        .map_err(rusqlite::Error::InvalidParameterName)?;

    struct NoteRow {
        mid: i64,
        flds: String,
    }

    let notes: Vec<NoteRow> = {
        let mut stmt = anki_conn.prepare("SELECT mid, flds FROM notes")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(NoteRow {
                    mid: row.get(0)?,
                    flds: row.get(1)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };

    let tx = conn.transaction()?;
    let new_deck = create_deck(deck_name, &*tx)?;

    let mut card_count = 0;

    for note in notes {
        let raw_fields: Vec<&str> = note.flds.split(FIELD_SEP).collect();
        if raw_fields.is_empty() {
            continue;
        }

        // A note's field order is defined by its note type, and one file can hold
        // several with different layouts, so the same ordinal means different
        // things across notes. Resolve the selected names per note type instead.
        let Some(layout) = models.get(&note.mid) else {
            continue;
        };

        let front_cloze = collect_fields(&front_fields, layout, &raw_fields);
        let back_cloze = collect_fields(&back_fields, layout, &raw_fields);
        let support_cloze = collect_fields(&support_fields, layout, &raw_fields);

        if front_cloze.trim().is_empty() && back_cloze.trim().is_empty() {
            continue;
        }

        let front_html = strip_event_handlers(&strip_scripts(&front_cloze));
        let back_html = strip_event_handlers(&strip_scripts(&back_cloze));
        let support_html = strip_event_handlers(&strip_scripts(&support_cloze));

        let mut card_media: HashMap<String, String> = HashMap::new();
        let front_clean = rewrite_media(&front_html, &sources, app_dir, &mut card_media);
        let back_clean = rewrite_media(&back_html, &sources, app_dir, &mut card_media);
        let support_clean = if support_html.trim().is_empty() {
            None
        } else {
            Some(rewrite_media(&support_html, &sources, app_dir, &mut card_media))
        };

        // imported_front/back are always Some, even when empty: the migration in
        // db.rs treats a NULL pair as an unmigrated row and would clobber front/back.
        let new_card = NewCard {
            group_id: new_deck.id,
            front: String::new(),
            back: String::new(),
            is_searchable: is_searchable,
            is_uploaded: true,
            support: None,
            imported_front: Some(front_clean),
            imported_back: Some(back_clean),
            imported_support: support_clean,
            front_image: None,
            back_image: None,
            front_audio: None,
            back_audio: None,
        };

        create_card_imported(new_card, &tx)?;
        card_count += 1;

        if create_flipped {
            // Fresh media map: the flipped copy gets its own file copies.
            let mut flipped_media: HashMap<String, String> = HashMap::new();
            let flipped_front = rewrite_media(&back_html, &sources, app_dir, &mut flipped_media);
            let flipped_back = rewrite_media(&front_html, &sources, app_dir, &mut flipped_media);
            let flipped_support = if support_html.trim().is_empty() {
                None
            } else {
                Some(rewrite_media(&support_html, &sources, app_dir, &mut flipped_media))
            };

            let flipped = NewCard {
                group_id: new_deck.id,
                front: String::new(),
                back: String::new(),
                is_searchable: is_searchable,
                is_uploaded: true,
                support: None,
                imported_front: Some(flipped_front),
                imported_back: Some(flipped_back),
                imported_support: flipped_support,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Two note types whose fields sit at different ordinals: "Romaji" is ord 1
    /// in one and ord 3 in the other. A positional mapping scrambles these.
    fn write_apkg(dir: &Path) -> String {
        let col_path = dir.join("collection.anki2");
        let conn = Connection::open(&col_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE col (id INTEGER PRIMARY KEY, models TEXT, decks TEXT);
             CREATE TABLE notes (id INTEGER PRIMARY KEY, mid INTEGER, flds TEXT);",
        )
        .unwrap();

        let models = serde_json::json!({
            "100": { "id": 100, "name": "Basic", "flds": [
                { "ord": 0, "name": "Number Hint" }, { "ord": 1, "name": "Romaji" },
                { "ord": 2, "name": "Kanji" }, { "ord": 3, "name": "Late" },
                { "ord": 4, "name": "Unused" },
            ]},
            "200": { "id": 200, "name": "Basique+", "flds": [
                { "ord": 0, "name": "Expression" }, { "ord": 1, "name": "Number" },
                { "ord": 2, "name": "Sound" }, { "ord": 3, "name": "Romaji" },
            ]},
        });
        let decks = serde_json::json!({
            "1": { "id": 1, "name": "Default" },
            "5": { "id": 5, "name": "Japanese numbers" },
        });
        conn.execute(
            "INSERT INTO col (id, models, decks) VALUES (1, ?1, ?2)",
            [models.to_string(), decks.to_string()],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO notes (id, mid, flds) VALUES (1, 100, ?1)",
            ["1 year old\x1fissai\x1f一歳\x1f\x1f"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO notes (id, mid, flds) VALUES (2, 200, ?1)",
            ["ゼロ\x1f0\x1f[sound:zero.mp3]\x1fzero"],
        )
        .unwrap();
        // "Late" is empty until here: a fixed-size sample window would miss it.
        conn.execute(
            "INSERT INTO notes (id, mid, flds) VALUES (3, 100, ?1)",
            ["2 years old\x1fni sai\x1f二歳\x1fshows up late\x1f"],
        )
        .unwrap();
        drop(conn);

        let apkg_path = dir.join("test.apkg");
        let file = std::fs::File::create(&apkg_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file::<_, ()>("collection.anki2", Default::default())
            .unwrap();
        zip.write_all(&std::fs::read(&col_path).unwrap()).unwrap();
        zip.start_file::<_, ()>("media", Default::default()).unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap();

        apkg_path.to_str().unwrap().to_string()
    }

    #[test]
    fn peek_unions_field_names_across_note_types() {
        let tmp = tempfile::tempdir().unwrap();
        let apkg = write_apkg(tmp.path());

        let peek = peek_anki_fields(&apkg).unwrap();
        let names: Vec<&str> = peek.fields.iter().map(|f| f.name.as_str()).collect();

        let field = |name: &str| peek.fields.iter().find(|f| f.name == name).unwrap();
        let texts = |name: &str| {
            field(name)
                .samples
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
        };

        assert_eq!(peek.total_notes, 3);
        assert!(names.contains(&"Kanji"));
        assert!(names.contains(&"Expression"));

        // Shared by both note types at different ordinals, so every note fills it.
        assert_eq!(field("Romaji").note_count, 3);
        assert_eq!(texts("Romaji"), vec!["issai", "zero", "ni sai"]);

        // Content that only appears in a later note still gets found.
        assert_eq!(field("Late").note_count, 1);
        assert_eq!(texts("Late"), vec!["shows up late"]);

        // Declared by the note type but never filled: worthless on a card, and
        // the count has to say so rather than claiming all of the note type's notes.
        assert_eq!(field("Unused").note_count, 0);
        assert!(field("Unused").samples.is_empty());

        // Media carries no text of its own; the UI describes it instead of printing it.
        let sound = &field("Sound").samples[0];
        assert!(sound.text.is_empty());
        assert_eq!(sound.media, vec!["audio"]);
    }

    #[test]
    fn samples_truncate_and_report_mixed_media() {
        let long = "ながい".repeat(80);
        let sample = preview_sample(&format!(
            "<div>{long}</div>[sound:x.mp3]<img src=\"y.jpg\">"
        ));

        assert_eq!(sample.media, vec!["audio", "image"]);
        assert!(sample.text.ends_with('…'));
        assert_eq!(sample.text.chars().count(), SAMPLE_MAX_CHARS + 1);
    }

    #[test]
    fn import_resolves_fields_per_note_type() {
        let tmp = tempfile::tempdir().unwrap();
        let apkg = write_apkg(tmp.path());
        let app_dir = tmp.path();

        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn, app_dir).unwrap();

        let (deck_id, count) = import_anki_deck(
            &apkg,
            app_dir,
            &mut conn,
            vec!["Romaji".to_string()],
            vec!["Kanji".to_string(), "Expression".to_string()],
            vec![],
            false,
            false,
        )
        .unwrap();
        assert_eq!(count, 3);

        let mut stmt = conn
            .prepare("SELECT imported_front, imported_back FROM card WHERE group_id = ?1 ORDER BY id")
            .unwrap();
        let cards: Vec<(String, String)> = stmt
            .query_map([deck_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        // Romaji is ord 1 on the Basic notes and ord 3 on the Basique+ one; all land
        // on the front. The back takes whichever of Kanji/Expression the note type has.
        assert_eq!(cards[0], ("issai".to_string(), "一歳".to_string()));
        assert_eq!(cards[1], ("zero".to_string(), "ゼロ".to_string()));
        assert_eq!(cards[2], ("ni sai".to_string(), "二歳".to_string()));
    }
}
