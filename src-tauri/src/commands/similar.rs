use crate::crud::*;
use crate::AppState;

#[derive(serde::Serialize)]
pub struct SimilarResult {
    front: Vec<Card>,
    back: Vec<Card>,
}

/// Similar cards: any is_searchable card in the same deck whose front matches a
/// front token OR whose back matches a back token of the studied card.
/// Partitioned front-matching first, back-only second.
#[tauri::command]
pub fn get_similar_cards(
    item_id: i64,
    state: tauri::State<AppState>,
) -> Result<SimilarResult, String> {
    let conn = state.conn.lock().unwrap();

    let (front, back, group_id): (String, String, i64) = conn
        .query_row(
            "SELECT front, back, group_id FROM card WHERE id = ?1",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| e.to_string())?;

    let front_tokens = extract_tokens(&front);
    let back_tokens = extract_tokens(&back);

    if front_tokens.is_empty() && back_tokens.is_empty() {
        return Ok(SimilarResult {
            front: vec![],
            back: vec![],
        });
    }

    let mut conditions: Vec<String> = Vec::new();
    let mut like_patterns: Vec<String> = Vec::new();
    for t in &front_tokens {
        conditions.push("front LIKE ?".to_string());
        like_patterns.push(format!("%{}%", t));
    }
    for t in &back_tokens {
        conditions.push("back LIKE ?".to_string());
        like_patterns.push(format!("%{}%", t));
    }

    let sql = format!(
        "SELECT id, group_id, front, back, support, imported_support, \
         front_image, back_image, front_audio, back_audio, \
         tier, ease, sequence, is_searchable, is_due, is_overdue, is_paused, is_uploaded, position \
         FROM card WHERE is_searchable = TRUE AND id != ? AND group_id = ? AND ({})",
        conditions.join(" OR ")
    );

    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(item_id), Box::new(group_id)];
    for p in like_patterns {
        params.push(Box::new(p));
    }
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|b| b.as_ref()).collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let results: Vec<Card> = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(Card {
                id: row.get(0)?,
                group_id: row.get(1)?,
                front: row.get(2)?,
                back: row.get(3)?,
                support: row.get(4)?,
                imported_support: row.get(5)?,
                front_image: row.get(6)?,
                back_image: row.get(7)?,
                front_audio: row.get(8)?,
                back_audio: row.get(9)?,
                tier: row.get(10)?,
                ease: row.get(11)?,
                sequence: row.get(12)?,
                is_searchable: row.get(13)?,
                is_due: row.get(14)?,
                is_overdue: row.get(15)?,
                is_paused: row.get(16)?,
                is_uploaded: row.get(17)?,
                position: row.get(18)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut front_matched: Vec<Card> = Vec::new();
    let mut back_only: Vec<Card> = Vec::new();
    for card in results {
        // Candidates are tokenized the same way as the studied card so both sides
        // compare HTML-free, entity-decoded, and paren-stripped.
        let cand_front = extract_tokens(&card.front);
        let cand_back = extract_tokens(&card.back);
        let fm = front_tokens
            .iter()
            .any(|t| cand_front.iter().any(|ct| word_boundary_match(ct, t)));
        let bm = !fm
            && back_tokens
                .iter()
                .any(|t| cand_back.iter().any(|ct| word_boundary_match(ct, t)));
        if fm {
            front_matched.push(card);
        } else if bm {
            back_only.push(card);
        }
        // else: SQL LIKE false-positive (e.g. "you" inside "younger") — discard
    }

    Ok(SimilarResult {
        front: front_matched,
        back: back_only,
    })
}

// Whole-word containment; prevents "you" matching "younger". Non-ASCII bytes
// (e.g. Japanese) count as word boundaries.
fn word_boundary_match(text: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let lowered = text.to_lowercase();
    let mut start = 0;
    while start < lowered.len() {
        match lowered[start..].find(token) {
            None => return false,
            Some(rel) => {
                let pos = start + rel;
                let before_ok = pos == 0 || !lowered.as_bytes()[pos - 1].is_ascii_alphanumeric();
                let end = pos + token.len();
                let after_ok =
                    end >= lowered.len() || !lowered.as_bytes()[end].is_ascii_alphanumeric();
                if before_ok && after_ok {
                    return true;
                }
                start = lowered[pos..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| pos + i)
                    .unwrap_or(lowered.len());
            }
        }
    }
    false
}

// Removes <tag …>…</tag> including content.
fn remove_tag_pair(s: &str, tag: &str) -> String {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let mut result = String::new();
    let mut rest = s;
    loop {
        match rest.find(open.as_str()) {
            None => {
                result.push_str(rest);
                break;
            }
            Some(start) => {
                result.push_str(&rest[..start]);
                rest = &rest[start..];
                match rest.find(close.as_str()) {
                    None => break,
                    Some(end) => rest = &rest[end + close.len()..],
                }
            }
        }
    }
    result
}

// Replaces each opening <tag …> with `replacement`; closing tags are left for strip_html.
fn replace_tag_with(s: &str, tag: &str, replacement: &str) -> String {
    let open = format!("<{}", tag);
    let mut result = String::new();
    let mut rest = s;
    loop {
        match rest.find(open.as_str()) {
            None => {
                result.push_str(rest);
                break;
            }
            Some(start) => {
                result.push_str(&rest[..start]);
                result.push_str(replacement);
                rest = &rest[start..];
                match rest.find('>') {
                    None => break,
                    Some(end) => rest = &rest[end + 1..],
                }
            }
        }
    }
    result
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
}

// Splits a card field into comma- or semicolon-separated tokens: br/hr/div
// become separators, remaining HTML is stripped, entities decoded,
// parenthetical text discarded.
fn extract_tokens(s: &str) -> Vec<String> {
    let s = remove_tag_pair(s, "audio");
    let s = replace_tag_with(&s, "br", ",");
    let s = replace_tag_with(&s, "hr", ",");
    let s = replace_tag_with(&s, "div", ",");
    let s = strip_html(&s);
    let s = decode_entities(&s);
    s.split([',', ';'])
        .map(|t| strip_parens(t).trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

fn strip_parens(s: &str) -> String {
    let mut result = String::new();
    let mut depth = 0usize;
    for ch in s.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ => {
                if depth == 0 {
                    result.push(ch);
                }
            }
        }
    }
    result
}

fn strip_html(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ => {
                if !in_tag {
                    result.push(ch);
                }
            }
        }
    }
    result
}
