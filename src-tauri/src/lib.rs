// NOTE: Notebooks as SRS groups is deprecated — some backend support remains,
// but the frontend UI no longer exposes it.

use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

pub mod app_utils;
pub mod commands;
pub mod crud;
mod db;

pub struct AppState {
    pub conn: Mutex<Connection>,
    pub app_dir: PathBuf,
}

// macOS 26+ composites bundled .icns icons onto a default grey squircle in the
// Dock. Setting the icon at runtime (which is what `tauri dev` does — hence the
// transparent toast in dev) makes the Dock and Cmd-Tab show the PNG as-is.
// Finder and Launchpad still show the bundled icon.
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else { return };
    let data = NSData::with_bytes(include_bytes!("../icons/icon.png"));
    if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
        unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .targets([tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("Toast".to_string()),
                    },
                )])
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            set_dock_icon();

            let app_dir = app
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");

            std::fs::create_dir_all(&app_dir).expect("failed to create app dir");

            let db_path = app_dir.join("database.db");

            let conn = Connection::open(db_path).expect("failed to open database");

            db::init_schema(&conn, &app_dir)?;

            log::info!("App started — db at {}", app_dir.display());

            app.manage(AppState {
                conn: Mutex::new(conn),
                app_dir: app_dir,
            });

            if let Some(window) = app.get_webview_window("main") {
                app_utils::media_permissions::allow_media_permissions(&window);
            }

            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::plans::create_plan,
            commands::plans::get_plans,
            commands::plans::get_plan_summaries,
            commands::plans::update_plan,
            commands::plans::delete_plan,
            commands::todos::create_todo,
            commands::todos::get_todos,
            commands::todos::update_todo,
            commands::todos::delete_todo,
            commands::todos::set_todo_position,
            commands::todos::get_todo_resources,
            commands::todos::set_todo_resources,
            commands::todos::get_todo_groups,
            commands::todos::set_todo_groups,
            commands::todos::complete_todo,
            commands::todos::uncomplete_todo,
            commands::todos::log_free_todo,
            commands::groups::get_groups,
            commands::groups::get_unassigned_groups,
            commands::groups::create_deck,
            commands::groups::merge_decks,
            commands::groups::get_decks,
            commands::groups::get_deck_card_counts,
            commands::groups::get_deck_srs_summaries,
            commands::groups::update_deck,
            commands::groups::delete_deck,
            commands::groups::create_notebook,
            commands::groups::merge_notebooks,
            commands::groups::get_notebooks,
            commands::groups::get_notebook_page_counts,
            commands::groups::update_notebook,
            commands::groups::delete_notebook,
            commands::cards::create_card,
            commands::cards::get_cards,
            commands::cards::update_card,
            commands::cards::delete_card,
            commands::cards::set_all_searchable,
            commands::cards::mark_for_review,
            commands::cards::prioritize_card,
            commands::cards::reset_deck,
            commands::cards::get_next_due_card,
            commands::cards::count_due_items,
            commands::cards::grade_item,
            commands::cards::get_card_grade_log,
            commands::cards::get_card_last_seen_dates,
            commands::pages::create_page,
            commands::pages::get_pages,
            commands::pages::update_page,
            commands::pages::delete_page,
            commands::pages::save_page_audio,
            commands::pages::read_audio_b64,
            commands::srs::add_group_to_plan,
            commands::srs::remove_group_from_plan,
            commands::srs::get_plan_srs_groups,
            commands::srs::update_scheduler,
            commands::srs::pause_all,
            commands::srs::unpause_all,
            commands::srs::clamp_group,
            commands::srs::max_clamp_group,
            commands::srs::add_group_time,
            commands::srs::update_date,
            commands::srs::get_current_date,
            commands::srs::is_day_stale,
            commands::resources::create_resource,
            commands::resources::get_resources,
            commands::resources::update_resource,
            commands::resources::delete_resource,
            commands::stats::get_plan_streak,
            commands::stats::get_group_stats,
            commands::stats::get_todo_stats,
            commands::stats::delete_group_stat,
            commands::stats::delete_group_stats_for_deck,
            commands::stats::delete_todo_stat,
            commands::stats::update_todo_stat,
            commands::stats::get_deleted_plan_ids,
            commands::stats::delete_deleted_plan_stats,
            commands::similar::get_similar_cards,
            commands::import::peek_anki_fields,
            commands::import::import_anki_deck,
            commands::import::cleanup_orphaned_media,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
