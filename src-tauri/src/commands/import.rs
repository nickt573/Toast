use crate::app_utils;
use crate::crud::*;
use crate::AppState;

#[tauri::command]
pub fn peek_anki_fields(path: String) -> Result<Vec<String>, String> {
    app_utils::import_anki::peek_anki_fields(&path)
}

#[tauri::command]
pub fn import_anki_deck(
    path: String,
    front_field_indices: Vec<usize>,
    back_field_indices: Vec<usize>,
    create_flipped: bool,
    is_searchable: bool,
    state: tauri::State<AppState>,
) -> Result<(i64, usize), String> {
    let mut conn = state.conn.lock().unwrap();
    let app_dir = state.app_dir.clone();
    app_utils::import_anki::import_anki_deck(
        &path,
        &app_dir,
        &mut conn,
        front_field_indices,
        back_field_indices,
        create_flipped,
        is_searchable,
    )
    .map_err(|e| {
        log::error!("import_anki_deck failed: {e}");
        e.to_string()
    })
}

#[tauri::command]
pub fn cleanup_orphaned_media(state: tauri::State<AppState>) -> Result<usize, String> {
    let conn = state.conn.lock().unwrap();
    let app_dir = state.app_dir.clone();
    delete::cleanup_orphaned_media(&conn, &app_dir).map_err(|e| e.to_string())
}
