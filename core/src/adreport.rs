//! Where the player was, from the lines `ad-report.js` (our Skyrim Platform
//! plugin) writes to `skyrim-platform.log`:
//!
//! `[ad-report] {"kind":"crumb"|"bug","at":...,"pos":[x,y,z],"cell":...,"cellName":...,"world":...,"near":[{"id","name","dist"}]}`
//!
//! A `crumb` is written every 20 seconds, so after a crash the report can still
//! say where the player was and who was near. A `bug` is written when the player
//! presses F10, and the launcher then offers to send a bug report.
//! A line cut off by a crash is skipped. Names and counts only: no paths.

use serde_json::Value;

const MARK: &str = "[ad-report] ";
/// The log is read from its end; a long session's start is not needed.
pub const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Collected {
    pub last_crumb: Option<Value>,
    pub last_bug: Option<Value>,
}

/// Every complete `[ad-report]` line, oldest first.
pub fn parse(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|line| line.find(MARK).map(|i| &line[i + MARK.len()..]))
        .filter_map(|json| serde_json::from_str::<Value>(json.trim()).ok())
        .filter(Value::is_object)
        .collect()
}

pub fn collect(text: &str) -> Collected {
    let mut c = Collected::default();
    for v in parse(text) {
        match v["kind"].as_str() {
            Some("bug") => c.last_bug = Some(v),
            Some("crumb") => c.last_crumb = Some(v),
            _ => {}
        }
    }
    c
}

fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() && *c != '\\' && *c != '/').take(60).collect::<String>().trim().to_string()
}

/// One line about a snapshot, or "none".
pub fn describe(s: Option<&Value>) -> String {
    let Some(s) = s else { return "none".into() };
    let pos = s["pos"].as_array().map(|p| p.iter().filter_map(Value::as_i64).map(|n| n.to_string()).collect::<Vec<_>>().join(",")).unwrap_or_default();
    let near: Vec<String> = s["near"]
        .as_array()
        .map(|n| {
            n.iter()
                .take(20)
                .map(|p| {
                    let who = p["name"].as_str().filter(|n| !n.is_empty()).or_else(|| p["id"].as_str()).unwrap_or("?");
                    format!("{} ({})", clean(who), p["dist"].as_i64().unwrap_or(0))
                })
                .collect()
        })
        .unwrap_or_default();
    format!(
        "{} pos {} cell {} world {} nearby: {}",
        s["at"].as_str().map(clean).unwrap_or_else(|| "?".into()),
        pos,
        clean(s["cellName"].as_str().or(s["cell"].as_str()).unwrap_or("?")),
        clean(s["world"].as_str().unwrap_or("?")),
        if near.is_empty() { "nobody".to_string() } else { near.join(", ") }
    )
}

/// The lines appended to a report: the last button press and the last known position.
pub fn position_text(log: &str) -> Option<String> {
    let c = collect(log);
    if c.last_bug.is_none() && c.last_crumb.is_none() {
        return None;
    }
    Some(format!("Last button press: {}\nLast known position: {}", describe(c.last_bug.as_ref()), describe(c.last_crumb.as_ref())))
}

/// When the newest bug snapshot was taken (its `at`), so a button press is
/// offered once and then marked as sent.
pub fn newest_bug_at(log: &str) -> Option<String> {
    collect(log).last_bug.and_then(|b| b["at"].as_str().map(str::to_string))
}

/// The text of a bug report: the player's note, then the snapshot lines.
pub fn bug_text(note: &str, log: &str) -> String {
    let note = note.trim();
    let mut out = Vec::new();
    if !note.is_empty() {
        out.push(format!("Player note: {note}"));
    }
    out.push(position_text(log).unwrap_or_else(|| "No position was recorded.".into()));
    out.join("\n").chars().take(15_000).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> String {
        [
            "[21:30:00:000] noise",
            r#"[21:30:10:000] [ad-report] {"kind":"crumb","at":"t1","pos":[1,2,3],"cell":"3c","cellName":"Whiterun","near":[{"id":"ff000a","name":"Dovah","dist":120}]}"#,
            r#"[21:30:30:000] [ad-report] {"kind":"bug","at":"t2","pos":[4,5,6],"near":[]}"#,
            r#"[21:30:50:000] [ad-report] {"kind":"crumb","at":"t3","pos":[7,8,9],"cellName":"Riften","near":[{"id":"ff000b","name":"Lydia2","dist":900}]}"#,
            r#"[21:31:00:000] [ad-report] {"kind":"crumb","at":"t4","pos":[1,"#,
        ]
        .join("\n")
    }

    #[test]
    fn the_newest_crumb_and_bug_are_found_and_a_torn_line_is_skipped() {
        let c = collect(&log());
        assert_eq!(c.last_crumb.unwrap()["at"], "t3");
        assert_eq!(c.last_bug.unwrap()["at"], "t2");
        assert_eq!(newest_bug_at(&log()).as_deref(), Some("t2"));
        assert_eq!(newest_bug_at("nothing here"), None);
    }

    #[test]
    fn the_report_names_where_the_player_was_and_who_was_near() {
        let t = position_text(&log()).unwrap();
        assert!(t.contains("Last known position: t3 pos 7,8,9 cell Riften world ? nearby: Lydia2 (900)"), "{t}");
        assert!(t.contains("Last button press: t2 pos 4,5,6 cell ? world ? nearby: nobody"), "{t}");
        assert_eq!(position_text("no report lines"), None);
    }

    #[test]
    fn a_bug_report_carries_the_note_and_never_a_path() {
        let snap = r#"[ad-report] {"kind":"bug","at":"C:\\Users\\Me\\x","pos":[1,2,3],"cellName":"A/B","near":[{"id":"ff1","name":"D:\\Evil\\Name","dist":5}]}"#;
        let t = bug_text("  the door stuck  ", snap);
        assert!(t.starts_with("Player note: the door stuck\n"), "{t}");
        assert!(!t.contains('\\') && !t.contains("C:\\"), "{t}");
        assert!(bug_text("", "x").contains("No position was recorded."));
        let long = bug_text(&"x".repeat(20_000), "x");
        assert_eq!(long.chars().count(), 15_000);
    }
}
