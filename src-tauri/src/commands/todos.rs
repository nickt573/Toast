use crate::crud::*;
use crate::AppState;

#[tauri::command]
pub fn create_todo(todo: NewTodo, state: tauri::State<AppState>) -> Result<Todo, String> {
    let mut conn = state.conn.lock().unwrap();
    create::create_todo(todo, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_todos(plan_id: i64, state: tauri::State<AppState>) -> Result<Vec<Todo>, String> {
    let mut conn = state.conn.lock().unwrap();
    read::get_todos(plan_id, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_todo(todo: Todo, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::update_todo(todo, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_todo(id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let mut conn = state.conn.lock().unwrap();
    delete::delete_todo(id, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_todo_position(
    todo_id: i64,
    position: Option<i64>,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let mut conn = state.conn.lock().unwrap();
    update::set_todo_position(todo_id, position, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_todo_resources(
    todo_id: i64,
    state: tauri::State<AppState>,
) -> Result<Vec<Resource>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_todo_resources(todo_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_todo_resources(
    todo_id: i64,
    resource_ids: Vec<i64>,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let mut conn = state.conn.lock().unwrap();
    update::set_todo_resources(todo_id, resource_ids, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_todo_groups(todo_id: i64, state: tauri::State<AppState>) -> Result<Vec<Group>, String> {
    let conn = state.conn.lock().unwrap();
    read::get_todo_groups(todo_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_todo_groups(
    todo_id: i64,
    group_ids: Vec<i64>,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let mut conn = state.conn.lock().unwrap();
    update::set_todo_groups(todo_id, group_ids, &mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn complete_todo(
    todo_id: i64,
    time_spent_minutes: f64,
    num_unit: Option<String>,
    details: Option<String>,
    resource_ids: Vec<i64>,
    group_ids: Vec<i64>,
    category: i64,
    text: Option<String>,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::complete_todo(
        todo_id,
        time_spent_minutes,
        num_unit,
        details,
        resource_ids,
        group_ids,
        category,
        text,
        &conn,
    )
    .map_err(|e| {
        log::error!("complete_todo(id={todo_id}) failed: {e}");
        e.to_string()
    })
}

#[tauri::command]
pub fn uncomplete_todo(todo_id: i64, state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::uncomplete_todo(todo_id, &conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn log_free_todo(
    plan_id: i64,
    text: String,
    category: i64,
    details: Option<String>,
    time_spent_minutes: f64,
    num_unit: Option<String>,
    group_ids: Vec<i64>,
    resource_ids: Vec<i64>,
    date: Option<String>,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    update::log_free_todo(
        plan_id,
        text,
        category,
        details,
        time_spent_minutes,
        num_unit,
        group_ids,
        resource_ids,
        date,
        &conn,
    )
    .map_err(|e| e.to_string())
}
