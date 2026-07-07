use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Plan {
    pub id: i64,
    pub name: String,
}

#[derive(Serialize, Deserialize)]
pub struct NewTodo {
    pub plan_id: i64,
    pub text: String,
    pub frequency: i64,
    pub category: i64,
}

#[derive(Serialize, Deserialize)]
pub struct Todo {
    pub id: i64,
    pub plan_id: i64,
    pub text: String,
    pub frequency: i64,
    pub category: i64,
    pub is_done: bool,
    pub is_disabled: bool,
    // Manual order; set via set_todo_position, never through update_todo.
    #[serde(default)]
    pub position: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub enum GroupType {
    #[serde(rename = "deck")]
    Deck,

    #[serde(rename = "notebook")]
    Notebook,
}

#[derive(Serialize, Deserialize)]
pub struct Group {
    pub id: i64,
    pub plan_id: Option<i64>,

    pub name: String,
    pub group_type: GroupType,
} // New Group would just be the name, as the Tauri call would separate the type

#[derive(Serialize, Deserialize, Clone)]
pub struct Card {
    pub id: i64,
    pub group_id: i64,

    pub front: String,
    pub back: String,

    pub tier: i32,
    pub ease: f32,
    pub sequence: i32,

    pub support: Option<String>,
    // Read-only support content mapped from Anki fields on import (Anki HTML).
    // Never written by update_card — only set at import time.
    #[serde(default)]
    pub imported_support: Option<String>,
    pub front_image: Option<String>,
    pub back_image: Option<String>,
    pub front_audio: Option<String>,
    pub back_audio: Option<String>,

    pub is_searchable: bool,
    pub is_uploaded: bool,

    pub is_due: bool,
    pub is_overdue: Option<bool>,
    pub is_paused: bool,

    pub position: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub struct NewCard {
    pub group_id: i64,

    pub front: String,
    pub back: String,

    pub is_searchable: bool,
    pub is_uploaded: bool,

    pub support: Option<String>,
    #[serde(default)]
    pub imported_support: Option<String>,
    pub front_image: Option<String>,
    pub back_image: Option<String>,
    pub front_audio: Option<String>,
    pub back_audio: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Page {
    pub id: i64,
    pub group_id: i64,
    pub title: String,
    pub description: Option<String>,
    pub content: String,
    pub audio_file: Option<String>,
    pub created_date: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NewPage {
    pub group_id: i64,
    pub title: String,
    pub description: Option<String>,
    pub content: String,
    pub audio_file: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Scheduler {
    pub group_id: i64,
    pub studied_new: i32,
    pub max_new: i32,
    pub studied_review: i32,
    pub max_review: i32,
    pub can_overflow: bool,
}

#[derive(Serialize, Deserialize)]
pub struct NewScheduler {
    pub group_id: i64,
    pub max_new: i32,
    pub max_review: i32,
    pub can_overflow: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Resource {
    pub id: i64,
    pub plan_id: i64,
    pub name: String,
    pub resource_type: Option<String>,
    pub url: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NewResource {
    pub plan_id: i64,
    pub name: String,
    pub resource_type: Option<String>,
    pub url: Option<String>,
    pub notes: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct GroupStat {
    pub id: i64,
    pub group_id: Option<i64>,
    pub plan_id: i64,
    pub plan_name: String,
    pub group_name: String,
    pub date: String,
    pub num_promote: i64,
    pub num_demote: i64,
    pub num_new: i64,
    pub time_spent_minutes: f64,
    pub retention_rate: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CardGradeLog {
    pub id: i64,
    pub card_id: i64,
    pub grade: i64,
    pub graded_at: String,
    pub old_tier: i64,
    pub new_tier: i64,
}

#[derive(Serialize, Deserialize)]
pub struct StatResource {
    pub name: String,
    pub url: Option<String>,
    pub resource_type: Option<String>,
    pub notes: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct TodoStatGroup {
    pub group_id: Option<i64>,
    pub name: String,
    pub group_type: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct TodoStat {
    pub id: i64,
    pub todo_id: Option<i64>,
    pub plan_id: i64,
    pub plan_name: String,
    pub date: String,
    pub text: String,
    pub category: String,
    pub details: Option<String>,
    pub time_spent_minutes: f64,
    pub num_unit: Option<String>,
    pub groups: Vec<TodoStatGroup>,
    pub resources: Vec<StatResource>,
}
