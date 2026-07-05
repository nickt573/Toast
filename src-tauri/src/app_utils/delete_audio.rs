use std::fs;

pub fn delete_card_audio_file(path: Option<String>) {
    if let Some(p) = path {
        if !p.is_empty() {
            let _ = fs::remove_file(&p);
        }
    }
}
