use crate::crud::*;
use crate::AppState;

#[tauri::command]
pub fn create_card(card: NewCard, state: tauri::State<AppState>) -> Result<Card, String> {
    let mut conn = state.conn.lock().unwrap();
    create::create_card(card, &mut conn, &state.app_dir).map_err(|e| {
        log::error!("create_card failed: {e}");
        e.to_string()
    })
}

#[tauri::command]
pub fn get_cards(deck_id: i64, state: tauri::State<AppState>) -> Result<Vec<Card>, String> {
    let mut conn = state.conn.lock().unwrap();
    read::get_cards(deck_id, &mut conn).map_err(|e| e.to_string())
}

// Returns the updated card: media paths are regenerated server-side.
#[tauri::command]
pub fn update_card(card: Card, state: tauri::State<AppState>) -> Result<Card, String> {
    let conn = state.conn.lock().unwrap();
    let id = card.id;
    update::update_card(card, &conn, &state.app_dir).map_err(|e| e.to_string())?;
    read::get_card(id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_card(id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    delete::delete_card(id, &conn, &state.app_dir).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_all_searchable(
    group_id: i64,
    searchable: bool,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::set_all_searchable(group_id, searchable, &conn).map_err(|e| e.to_string())
}

// Both return the updated card: the new sequence is derived server-side, and
// prioritize_card's fill_group may also have made the card due.
#[tauri::command]
pub fn mark_for_review(card_id: i64, state: tauri::State<AppState>) -> Result<Card, String> {
    let conn = state.conn.lock().unwrap();
    scheduling::mark_for_review(card_id, &conn).map_err(|e| e.to_string())?;
    read::get_card(card_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn prioritize_card(card_id: i64, state: tauri::State<AppState>) -> Result<Card, String> {
    let conn = state.conn.lock().unwrap();
    scheduling::prioritize_card(card_id, &conn).map_err(|e| e.to_string())?;
    read::get_card(card_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reset_deck(group_id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    scheduling::reset_deck(group_id, &conn).map_err(|e| e.to_string())
}

/// Swapping a card out mid-session: pausing it frees its slot, so the queue refills
/// with an eligible card from the same track.
#[tauri::command]
pub fn set_card_paused(
    card_id: i64,
    paused: bool,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::set_card_paused(card_id, paused, &conn).map_err(|e| e.to_string())
}

/// Archives every stat row a deck has, in every plan. Offered after a reset.
#[tauri::command]
pub fn archive_deck_stats(group_id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    scheduling::archive_deck_stats(group_id, &conn).map_err(|e| e.to_string())
}

/// Fetch one random is_due item for a group session.
#[tauri::command]
pub fn get_next_due_card(
    group_id: i64,
    exclude_id: Option<i64>,
    state: tauri::State<AppState>,
) -> Result<Option<Card>, String> {
    let conn = state.conn.lock().unwrap();

    let exclude_clause = if exclude_id.is_some() {
        "AND id != ?2"
    } else {
        ""
    };

    let result = conn.query_row(
        &format!(
            r#"
            SELECT id, group_id, front, back, support,
                   imported_front, imported_back, imported_support,
                   front_image, back_image, front_audio, back_audio,
                   tier, ease, sequence, is_searchable, is_due, is_overdue, is_paused, is_uploaded,
                   position
            FROM card
            WHERE group_id = ?1 AND is_due = TRUE AND is_paused = FALSE {exclude_clause}
            ORDER BY RANDOM()
            LIMIT 1
            "#
        ),
        rusqlite::params_from_iter(std::iter::once(group_id as i64).chain(exclude_id.into_iter())),
        |row| {
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
        },
    );
    match result {
        Ok(card) => Ok(Some(card)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn count_due_items(group_id: i64, state: tauri::State<AppState>) -> Result<(i64, i64), String> {
    let conn = state.conn.lock().unwrap();
    scheduling::count_due_items(&group_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn grade_item(item_id: i64, grade: u8, state: tauri::State<AppState>) -> Result<(), String> {
    let mut conn = state.conn.lock().unwrap();
    scheduling::grade_item(item_id, grade, &mut conn).map_err(|e| {
        log::error!("grade_item(id={item_id}) failed: {e}");
        e.to_string()
    })
}

#[tauri::command]
pub fn get_card_grade_log(
    card_id: i64,
    state: tauri::State<AppState>,
) -> Result<Vec<CardGradeLog>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_card_grade_log(card_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_card_last_seen_dates(
    deck_id: i64,
    state: tauri::State<AppState>,
) -> Result<Vec<(i64, String)>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_card_last_seen_dates(deck_id, &conn).map_err(|e| e.to_string())
}
