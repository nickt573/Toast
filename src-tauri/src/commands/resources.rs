use crate::crud::*;
use crate::AppState;

#[tauri::command]
pub fn create_resource(
    resource: NewResource,
    state: tauri::State<AppState>,
) -> Result<Resource, String> {
    let conn = state.conn.lock().unwrap();
    create::create_resource(resource, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_resources(plan_id: i64, state: tauri::State<AppState>) -> Result<Vec<Resource>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_resources(plan_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_resource(resource: Resource, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::update_resource(resource, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_resource(id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    delete::delete_resource(id, &conn).map_err(|e| e.to_string())
}

