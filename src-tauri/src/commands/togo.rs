use crate::app_utils::togo::{self, CloseBehavior, SlotInfo, ToGoConfig};
use crate::crud::delete;
use crate::AppState;

/// Canonical lowercase-hyphenated UUIDv4, matching what the Worker accepts.
fn valid_id(id: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(id.trim())
        .ok()
        .filter(|u| u.get_version_num() == 4)
        .map(|u| u.to_string())
        .ok_or_else(|| "That doesn't look like a Toast to Go ID.".to_string())
}

#[tauri::command]
pub fn get_togo_config(state: tauri::State<AppState>) -> Result<ToGoConfig, String> {
    togo::load_config(&state.app_dir)
}

#[tauri::command]
pub fn set_close_behavior(
    behavior: CloseBehavior,
    state: tauri::State<AppState>,
) -> Result<ToGoConfig, String> {
    let mut cfg = togo::load_config(&state.app_dir)?;
    cfg.close_behavior = behavior;
    togo::save_config(&state.app_dir, &cfg)?;
    Ok(cfg)
}

#[tauri::command]
pub fn label_recent_pull(
    id: String,
    label: Option<String>,
    state: tauri::State<AppState>,
) -> Result<ToGoConfig, String> {
    let mut cfg = togo::load_config(&state.app_dir)?;
    if let Some(entry) = cfg.recent_pulls.iter_mut().find(|r| r.id == id) {
        entry.label = label.filter(|l| !l.trim().is_empty());
    }
    togo::save_config(&state.app_dir, &cfg)?;
    Ok(cfg)
}

#[tauri::command]
pub fn forget_recent_pull(id: String, state: tauri::State<AppState>) -> Result<ToGoConfig, String> {
    let mut cfg = togo::load_config(&state.app_dir)?;
    cfg.recent_pulls.retain(|r| r.id != id);
    togo::save_config(&state.app_dir, &cfg)?;
    Ok(cfg)
}

#[tauri::command]
pub async fn slot_exists(id: String) -> Result<Option<SlotInfo>, String> {
    let id = valid_id(&id)?;
    togo::slot_info(&id).await
}

/// Bundles this machine and overwrites its slot. Returns the push time.
/// `force` skips the cooldown (push-on-close must never be silently dropped).
#[tauri::command]
pub async fn push_package(
    force: Option<bool>,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let app_dir = state.app_dir.clone();
    let cfg = togo::load_config(&app_dir)?;

    let cooldown = togo::push_cooldown(&cfg);
    if cooldown > 0 && force != Some(true) {
        return Err(format!("You just pushed. Try again in {cooldown}s."));
    }
    let id = cfg.instance_id;

    // Scoped: the guard isn't Send and must not be held across an await.
    let (_tmp, zip_path) = {
        let conn = state.conn.lock().unwrap();
        let _ = delete::cleanup_orphaned_media(&conn, &app_dir);
        togo::bundle(&app_dir, &conn, &id)?
    };

    togo::upload(&zip_path, &id).await?;

    let now = chrono::Local::now().to_rfc3339();
    let mut cfg = togo::load_config(&app_dir)?;
    cfg.last_push = Some(now.clone());
    togo::save_config(&app_dir, &cfg)?;
    Ok(now)
}

/// Replaces all local data with the package at `id`. Destructive.
#[tauri::command]
pub async fn pull_package(id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let id = valid_id(&id)?;
    let app_dir = state.app_dir.clone();

    let (_tmp, zip_path) = togo::download(&id).await?;

    {
        let mut conn = state.conn.lock().unwrap();
        togo::restore(&app_dir, &zip_path, &mut conn)?;
    }

    // The pull already succeeded; failing to record it must not fail it.
    if let Err(e) = togo::load_config(&app_dir).and_then(|mut cfg| {
        togo::record_pull(&mut cfg, &id);
        togo::save_config(&app_dir, &cfg)
    }) {
        log::error!("pull succeeded but couldn't be recorded: {e}");
    }
    Ok(())
}
