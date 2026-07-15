/// Collects local `src` values from <audio> tags in card HTML.
/// Only handles double-quoted `src="…"`, single-quoted attributes are missed
/// (import always writes double quotes, so only hand-edited HTML is affected).
pub fn extract_audio_paths_from_html(html: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let lower_html = html.to_lowercase();
    let mut search_start = 0;

    while let Some(rel_start) = lower_html[search_start..].find("<audio") {
        let start = search_start + rel_start;
        let after_tag = &html[start..];
        let lower_after = &lower_html[start..];

        if let Some(src_start) = lower_after.find("src=\"") {
            let after_src = &after_tag[src_start + 5..];
            if let Some(end) = after_src.find('"') {
                let path = &after_src[..end];
                if !path.starts_with("http") && !path.starts_with("asset://") {
                    paths.push(path.to_string());
                }
            }
        }
        search_start = start + 6;
    }
    paths
}
