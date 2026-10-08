//! "What's new" for the update bar: the lines staff put in the manifest, or,
//! when there are none, what is about to change on this PC. Shown beside the
//! Update button; it never blocks anything.

/// Most lines shown, and the longest line.
const MAX_LINES: usize = 8;
const MAX_LINE: usize = 160;

fn clean(line: &str) -> Option<String> {
    let s: String = line.chars().filter(|c| !c.is_control()).take(MAX_LINE).collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// A file's name for the player: the last path part without its folder.
fn short(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// `notes` are the manifest's lines; `download` the paths about to be
/// downloaded and `remove` the paths about to go. With notes, only the notes.
pub fn lines(notes: &[String], download: &[String], remove: &[String]) -> Vec<String> {
    let given: Vec<String> = notes.iter().filter_map(|n| clean(n)).take(MAX_LINES).collect();
    if !given.is_empty() {
        return given;
    }
    let mut out = Vec::new();
    let mut add = |verb: &str, paths: &[String]| {
        if paths.is_empty() {
            return;
        }
        let mut names: Vec<&str> = paths.iter().map(|p| short(p)).collect();
        names.sort_unstable();
        names.dedup();
        let shown = names.iter().take(4).copied().collect::<Vec<_>>().join(", ");
        let more = if names.len() > 4 { format!(" and {} more", names.len() - 4) } else { String::new() };
        out.push(format!("{verb} {shown}{more}"));
    };
    add("Updated:", download);
    add("Removed:", remove);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn staff_notes_win_and_are_trimmed() {
        let long = "x".repeat(300);
        let out = lines(&[" New tavern in Whiterun ".into(), "".into(), long], &v(&["Data/a.esp"]), &[]);
        assert_eq!(out[0], "New tavern in Whiterun");
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].chars().count(), 160);
        assert!(!out.iter().any(|l| l.contains("a.esp")));
    }

    #[test]
    fn at_most_eight_notes() {
        let many: Vec<String> = (0..20).map(|i| format!("note {i}")).collect();
        assert_eq!(lines(&many, &[], &[]).len(), 8);
    }

    #[test]
    fn without_notes_it_names_what_changes_by_file_name() {
        let out = lines(&[], &v(&["Data/Meshes/b.nif", "Data/a.esp", "Data/c.esp", "Data/d.esp", "Data/e.esp", "Data/f.esp"]), &v(&["Data\\old.esp"]));
        assert_eq!(out, ["Updated: a.esp, b.nif, c.esp, d.esp and 2 more", "Removed: old.esp"]);
        assert!(lines(&[], &[], &[]).is_empty());
    }
}
