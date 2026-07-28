use crate::crud::*;
use crate::AppState;

#[tauri::command]
pub fn get_units(state: tauri::State<AppState>) -> Result<Vec<Unit>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_units(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_unit(names: Vec<String>, state: tauri::State<AppState>) -> Result<i64, String> {
    let conn = state.conn.lock().unwrap();
    update::create_unit(names, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_variant(group_id: i64, name: String, state: tauri::State<AppState>) -> Result<i64, String> {
    let conn = state.conn.lock().unwrap();
    update::add_variant(group_id, &name, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rename_variant(id: i64, name: String, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::rename_variant(id, &name, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_main_variant(id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::set_main_variant(id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn merge_units(from_group: i64, into_group: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::merge_units(from_group, into_group, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_variant(id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    delete::delete_variant(id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_unit(group_id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    delete::delete_unit(group_id, &conn).map_err(|e| e.to_string())
}
