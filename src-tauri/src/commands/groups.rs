use crate::crud::*;
use crate::AppState;

#[tauri::command]
pub fn get_groups(state: tauri::State<AppState>) -> Result<Vec<Group>, String> {
    let mut conn = state.conn.lock().unwrap();
    read::get_groups(&mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_unassigned_groups(state: tauri::State<AppState>) -> Result<Vec<Group>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_unassigned_groups(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_deck(name: String, state: tauri::State<AppState>) -> Result<Group, String> {
    let mut conn = state.conn.lock().unwrap();
    create::create_deck(name, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn merge_decks(
    deck_a_id: i64,
    deck_b_id: i64,
    new_name: String,
    reset: bool,
    state: tauri::State<AppState>,
) -> Result<Group, String> {
    let mut conn = state.conn.lock().unwrap();
    create::merge_decks(deck_a_id, deck_b_id, new_name, reset, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_decks(state: tauri::State<AppState>) -> Result<Vec<Group>, String> {
    let mut conn = state.conn.lock().unwrap();
    read::get_decks(&mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_deck_card_counts(state: tauri::State<AppState>) -> Result<Vec<(i64, i64)>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_deck_card_counts(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_deck_srs_summaries(
    state: tauri::State<AppState>,
) -> Result<Vec<(i64, i64, i64)>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_deck_srs_summaries(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_deck(deck: Group, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::update_deck(deck, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_deck(id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    delete::delete_deck(id, &conn, &state.app_dir).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_notebook(name: String, state: tauri::State<AppState>) -> Result<Group, String> {
    let mut conn = state.conn.lock().unwrap();
    create::create_notebook(name, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn merge_notebooks(
    notebook_a_id: i64,
    notebook_b_id: i64,
    new_name: String,
    state: tauri::State<AppState>,
) -> Result<Group, String> {
    let mut conn = state.conn.lock().unwrap();
    create::merge_notebooks(notebook_a_id, notebook_b_id, new_name, &mut conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_notebooks(state: tauri::State<AppState>) -> Result<Vec<Group>, String> {
    let mut conn = state.conn.lock().unwrap();
    read::get_notebooks(&mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_notebook_page_counts(state: tauri::State<AppState>) -> Result<Vec<(i64, i64)>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_notebook_page_counts(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_notebook(notebook: Group, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::update_notebook(notebook, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_notebook(id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    delete::delete_notebook(id, &conn, &state.app_dir).map_err(|e| e.to_string())
}
