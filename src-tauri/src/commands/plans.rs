use crate::crud::*;
use crate::AppState;

#[tauri::command]
pub fn create_plan(name: &str, state: tauri::State<AppState>) -> Result<Plan, String> {
    let mut conn = state.conn.lock().unwrap();
    create::create_plan(name, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_plans(state: tauri::State<AppState>) -> Result<Vec<Plan>, String> {
    let mut conn = state.conn.lock().unwrap();
    read::get_plans(&mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_plan_summaries(
    state: tauri::State<AppState>,
) -> Result<Vec<(i64, i64, i64, i64)>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_plan_summaries(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_plan(id: i64, name: String, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::update_plan(id, name, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_plan(id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let mut conn = state.conn.lock().unwrap();
    delete::delete_plan(id, &mut conn).map_err(|e| e.to_string())
}
