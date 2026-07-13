use crate::crud::*;
use crate::AppState;

#[tauri::command]
pub fn add_group_to_plan(
    group_id: i64,
    plan_id: i64,
    scheduler: NewScheduler,
    state: tauri::State<AppState>,
) -> Result<Scheduler, String> {
    let mut conn = state.conn.lock().unwrap();
    create::add_group_to_plan(group_id, plan_id, scheduler, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_group_from_plan(
    group_id: i64,
    reset: bool,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let mut conn = state.conn.lock().unwrap();
    delete::remove_group_from_plan(group_id, reset, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_plan_srs_groups(
    plan_id: i64,
    state: tauri::State<AppState>,
) -> Result<Vec<(Group, Scheduler)>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_plan_srs_groups(plan_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_scheduler(scheduler: Scheduler, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    scheduling::update_scheduler(scheduler, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pause_all(group_id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    scheduling::pause_all(group_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn unpause_all(group_id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    scheduling::unpause_all(group_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clamp_group(group_id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    scheduling::clamp_group(group_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn max_clamp_group(group_id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    scheduling::max_clamp_group(group_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_group_time(
    group_id: i64,
    minutes: f64,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    scheduling::add_group_time(group_id, minutes, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_date(state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    scheduling::update_date(&conn).map_err(|e| {
        log::error!("update_date failed: {e}");
        e.to_string()
    })
}

#[tauri::command]
pub fn get_current_date(state: tauri::State<AppState>) -> Result<String, String> {
    let conn = state.conn.lock().unwrap();
    scheduling::get_date(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn is_day_stale(state: tauri::State<AppState>) -> Result<bool, String> {
    let conn = state.conn.lock().unwrap();
    let stored = scheduling::get_date(&conn).map_err(|e| e.to_string())?;
    Ok(stored != chrono::Local::now().date_naive().to_string())
}
