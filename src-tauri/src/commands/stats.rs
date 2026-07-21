use crate::crud::*;
use crate::AppState;

#[derive(serde::Serialize)]
pub struct StreakInfo {
    streak: i64,
    studied_today: bool,
}

#[tauri::command]
pub fn get_plan_streak(plan_id: i64, state: tauri::State<AppState>) -> Result<StreakInfo, String> {
    let conn = state.conn.lock().unwrap();
    let (streak, studied_today) =
        scheduling::get_plan_streak(plan_id, &conn).map_err(|e| e.to_string())?;
    Ok(StreakInfo {
        streak,
        studied_today,
    })
}

#[tauri::command]
pub fn get_group_stats(
    plan_id: i64,
    state: tauri::State<AppState>,
) -> Result<Vec<GroupStat>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_group_stats(plan_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_todo_stats(
    plan_id: i64,
    state: tauri::State<AppState>,
) -> Result<Vec<TodoStat>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_todo_stats(plan_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_group_stat(id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    delete::delete_group_stat(id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_group_stats(ids: Vec<i64>, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    delete::delete_group_stats(&ids, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_group_stat_archived(
    id: i64,
    archived: bool,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::set_group_stat_archived(id, archived, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_group_stats_archived(
    ids: Vec<i64>,
    archived: bool,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::set_group_stats_archived(&ids, archived, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_todo_stat(id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    delete::delete_todo_stat(id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_todo_stat(
    id: i64,
    text: String,
    category: i64,
    details: Option<String>,
    time_spent_minutes: f64,
    num_unit: Option<String>,
    remove_group_row_ids: Vec<i64>,
    remove_resource_row_ids: Vec<i64>,
    add_group_ids: Vec<i64>,
    add_resource_ids: Vec<i64>,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::update_todo_stat(
        id,
        text,
        category,
        details,
        time_spent_minutes,
        num_unit,
        remove_group_row_ids,
        remove_resource_row_ids,
        add_group_ids,
        add_resource_ids,
        &conn,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_deleted_plan_ids(state: tauri::State<AppState>) -> Result<Vec<(i64, String)>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_deleted_plan_ids(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_deleted_plan_stats(
    plan_id: i64,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    delete::delete_deleted_plan_stats(plan_id, &conn).map_err(|e| e.to_string())
}
