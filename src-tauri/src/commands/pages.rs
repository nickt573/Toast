use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;

use crate::crud::*;
use crate::AppState;

#[tauri::command]
pub fn create_page(page: NewPage, state: tauri::State<AppState>) -> Result<Page, String> {
    let mut conn = state.conn.lock().unwrap();
    create::create_page(page, &mut conn, &state.app_dir).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_pages(notebook_id: i64, state: tauri::State<AppState>) -> Result<Vec<Page>, String> {
    let mut conn = state.conn.lock().unwrap();
    read::get_pages(notebook_id, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_page(page: Page, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::update_page(page, &conn, &state.app_dir).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_page(id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    delete::delete_page(id, &conn, &state.app_dir).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_page_audio(data: Vec<u8>, state: tauri::State<AppState>) -> Result<String, String> {
    let app_dir = &state.app_dir;
    let audio_dir = app_dir.join("pages").join("audio");
    std::fs::create_dir_all(&audio_dir).map_err(|e| e.to_string())?;

    let filename = format!("{}.mp4", uuid::Uuid::new_v4());
    let path = audio_dir.join(&filename);
    std::fs::write(&path, &data).map_err(|e| e.to_string())?;

    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn delete_page_audio(path: String) -> Result<(), String> {
    std::fs::remove_file(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn read_audio_b64(path: String) -> Result<String, String> {
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    Ok(BASE64_STANDARD.encode(&bytes))
}
