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

    let (front, back, imported_front, imported_back, group_id): (
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
    ) = conn
        .query_row(
            "SELECT front, back, imported_front, imported_back, group_id FROM card WHERE id = ?1",
            [item_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;

    let front_tokens = side_tokens(&front, imported_front.as_deref());
    let back_tokens = side_tokens(&back, imported_back.as_deref());

    if front_tokens.is_empty() && back_tokens.is_empty() {
        return Ok(SimilarResult {
            front: vec![],
            back: vec![],
        });
    }

    // No SQL text prefilter: raw columns hold HTML with entities (&nbsp; etc.)
    // so LIKE against decoded tokens drops valid matches. Matching is done on tokenized text below.
    let sql = "SELECT id, group_id, front, back, support, imported_front, imported_back, imported_support, \
         front_image, back_image, front_audio, back_audio, \
         tier, ease, sequence, is_searchable, is_due, is_overdue, is_paused, is_uploaded, position \
         FROM card WHERE is_searchable = TRUE AND id != ?1 AND group_id = ?2";

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let results: Vec<Card> = stmt
        .query_map([item_id, group_id], |row| {
            Ok(Card {
                id: row.get(0)?,
                group_id: row.get(1)?,
                front: row.get(2)?,
                back: row.get(3)?,
                support: row.get(4)?,
                imported_front: row.get(5)?,
                imported_back: row.get(6)?,
                imported_support: row.get(7)?,
                front_image: row.get(8)?,
                back_image: row.get(9)?,
                front_audio: row.get(10)?,
                back_audio: row.get(11)?,
                tier: row.get(12)?,
                ease: row.get(13)?,
                sequence: row.get(14)?,
                is_searchable: row.get(15)?,
                is_due: row.get(16)?,
                is_overdue: row.get(17)?,
                is_paused: row.get(18)?,
                is_uploaded: row.get(19)?,
                position: row.get(20)?,
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
        let cand_front = side_tokens(&card.front, card.imported_front.as_deref());
        let cand_back = side_tokens(&card.back, card.imported_back.as_deref());
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
    }

    Ok(SimilarResult {
        front: front_matched,
        back: back_only,
    })
}

// One side's search terms: user text plus any imported Anki HTML.
fn side_tokens(user: &str, imported: Option<&str>) -> Vec<String> {
    let mut tokens = extract_tokens(user);
    if let Some(html) = imported {
        tokens.extend(extract_tokens(html));
    }
    tokens
}

// Whole-word containment: prevents "you" matching "younger". Non-ASCII bytes
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

// Decodes common named entities plus any numeric entity (&#39;, &#xa0;, ...).
// Unrecognized entities pass through unchanged.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        // Entity names are short, a distant ';' means this '&' is literal text
        let end = rest.find(';').filter(|&end| end <= 12);
        let decoded = end.and_then(|end| {
            let name = &rest[1..end];
            match name {
                "nbsp" => Some((' ', end)),
                "amp" => Some(('&', end)),
                "lt" => Some(('<', end)),
                "gt" => Some(('>', end)),
                "quot" => Some(('"', end)),
                "apos" => Some(('\'', end)),
                _ => name
                    .strip_prefix('#')
                    .and_then(|num| {
                        if let Some(hex) = num.strip_prefix('x').or(num.strip_prefix('X')) {
                            u32::from_str_radix(hex, 16).ok()
                        } else {
                            num.parse::<u32>().ok()
                        }
                    })
                    .and_then(char::from_u32)
                    .map(|ch| (ch, end)),
            }
        });
        match decoded {
            Some((ch, end)) => {
                out.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// Same canonical form as normalizeSearchText in CardFace.jsx: invisible
// characters removed, curly quotes straightened, whitespace collapsed.
fn normalize_text(s: &str) -> String {
    let mapped: String = s
        .chars()
        .filter(|c| !matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200D}' | '\u{FEFF}'))
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            c if c.is_whitespace() => ' ',
            c => c,
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
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
        .map(|t| normalize_text(&strip_parens(t)).to_lowercase())
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
