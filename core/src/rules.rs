//! `launcher-rules.json`: the Play-time checks and load-order policy staff can
//! change on the server without a launcher release.
//!
//! The file sits next to `mods.json` under the server's launcher address and
//! is signed like the other feeds (`feedsig::Feed::Rules`). Each Play fetches
//! it once, keeps the verified copy in the launcher's data folder, and uses
//! that copy when the server can't be reached. With neither, the built-in
//! defaults apply, which are exactly what the launcher did before this file
//! existed. The file can only switch things off, soften them, or allow more;
//! it cannot download, run or delete anything. Format: docs/launcher-rules.md.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::feedsig::{Feed, Trust};
use crate::health::{Report, Status};
use crate::netcheck::Flags;

const SCHEMA: u64 = 1;
const CACHE_FILE: &str = "launcher-rules.json";
const MAX_TEXT: usize = 400;
const MAX_LIST: usize = 100;

/// What a plugin-order mismatch does to Play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Play stops (the default, as before).
    Fail,
    /// Shown in the health report; Play goes on.
    Warn,
    /// Not reported.
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Server,
    Cache,
    BuiltIn,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    pub rev: String,
    pub flags: Flags,
    pub server_order: Level,
    /// Plugin names whose order/size/crc mismatches the order check skips.
    pub order_ignore: Vec<String>,
    /// Empty Skyrim.ccc for the session when the server loads more than the base masters.
    pub empty_ccc: bool,
    /// Rewrite loadorder.txt to the server's list on Play.
    pub rewrite_loadorder: bool,
    /// Extra game-folder paths (e.g. `Data/Foo.esp`) the no-mods check leaves alone.
    pub allow_files: Vec<String>,
    /// Replacement for the "plugins don't match" message; `{which}` and `{tell}` are filled in.
    pub order_mismatch_text: Option<String>,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            rev: "built-in".into(),
            flags: Flags::default(),
            server_order: Level::Fail,
            order_ignore: Vec::new(),
            empty_ccc: true,
            rewrite_loadorder: true,
            allow_files: Vec::new(),
            order_mismatch_text: None,
        }
    }
}

fn clean_text(s: &str) -> Option<String> {
    let t: String = s.chars().filter(|c| !c.is_control()).collect();
    let t = t.trim();
    (!t.is_empty() && t.chars().count() <= MAX_TEXT).then(|| t.to_string())
}

impl Rules {
    /// Reads the served file. A key that is missing or has the wrong type keeps
    /// its default; a file from a newer schema is refused (use the cache or defaults).
    pub fn parse(bytes: &[u8]) -> Result<Rules, String> {
        let v: Value = serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)).map_err(|e| format!("not valid JSON: {e}"))?;
        let schema = v.get("schema").and_then(Value::as_u64).unwrap_or(SCHEMA);
        if schema != SCHEMA {
            return Err(format!("schema {schema}, this launcher reads {SCHEMA}"));
        }
        let mut r = Rules { rev: v.get("rev").and_then(Value::as_str).and_then(clean_text).unwrap_or_else(|| "unnamed".into()), ..Rules::default() };
        // Kill switch: everything below is ignored and the built-in behaviour applies.
        if v.get("off").and_then(Value::as_bool) == Some(true) {
            return Ok(r);
        }
        let section = |name: &str| v.get(name).cloned().unwrap_or(Value::Null);
        let (checks, order, allow, text) = (section("checks"), section("order"), section("allow"), section("text"));
        let on = |o: &Value, k: &str| o.get(k).and_then(Value::as_bool) != Some(false);
        r.flags = Flags {
            firewall_fix: on(&checks, "firewallFix"),
            udp_probe: on(&checks, "udpProbe"),
            connect_watch: on(&checks, "connectWatch"),
            platform_guard: on(&checks, "platformGuard"),
            session_upload: on(&checks, "sessionUpload"),
            asset_report: on(&checks, "assetReport"),
        };
        r.server_order = match checks.get("serverOrder").and_then(Value::as_str) {
            Some("warn") => Level::Warn,
            Some("off") => Level::Off,
            _ => Level::Fail,
        };
        r.empty_ccc = on(&order, "emptyCcc");
        r.rewrite_loadorder = on(&order, "rewriteLoadorderTxt");
        let strings = |o: &Value, k: &str| -> Vec<String> {
            o.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).take(MAX_LIST).collect()).unwrap_or_default()
        };
        r.order_ignore = strings(&order, "ignore").into_iter().filter(|n| crate::serverorder::plain_plugin_name(n)).collect();
        r.allow_files = strings(&allow, "files")
            .into_iter()
            .filter(|p| !p.contains('\\') && crate::manifest::safe_relative(p).is_ok())
            .collect();
        r.order_mismatch_text = text.get("orderMismatch").and_then(Value::as_str).and_then(clean_text);
        Ok(r)
    }

    /// The status.json switches, further narrowed by this file: a part is on
    /// only when neither turns it off.
    pub fn narrow(&self, status: Flags) -> Flags {
        Flags {
            firewall_fix: status.firewall_fix && self.flags.firewall_fix,
            udp_probe: status.udp_probe && self.flags.udp_probe,
            connect_watch: status.connect_watch && self.flags.connect_watch,
            platform_guard: status.platform_guard && self.flags.platform_guard,
            session_upload: status.session_upload && self.flags.session_upload,
            asset_report: status.asset_report && self.flags.asset_report,
        }
    }

    /// Applies the order rules to the health report's "serverorder" check: skips
    /// the ignored plugins, then the level (warn keeps the finding but lets Play
    /// go on; off hides it). Returns true when the check no longer stops Play.
    pub fn adjust_order_check(&self, report: &mut Report) {
        let ignore: Vec<String> = self.order_ignore.iter().map(|n| n.to_ascii_lowercase()).collect();
        for c in report.checks.iter_mut().filter(|c| c.id == "serverorder" && c.status == Status::Fail) {
            c.items.retain(|i| {
                let l = i.to_ascii_lowercase();
                !ignore.iter().any(|n| l.contains(n.as_str()))
            });
            if c.items.is_empty() {
                c.status = Status::Info;
                c.detail = "The server's launcher rules skip the plugins that differ.".into();
            } else {
                match self.server_order {
                    Level::Fail => {}
                    Level::Warn => c.status = Status::Warn,
                    Level::Off => {
                        c.status = Status::Info;
                        c.items.clear();
                        c.detail = "The server's launcher rules switched the plugin order check off.".into();
                    }
                }
            }
        }
        report.worst = report.checks.iter().map(|c| c.status).max().unwrap_or(Status::Ok);
    }

    /// The message when the order check stops Play.
    pub fn order_mismatch_message(&self, which: &str, tell: &str) -> String {
        match &self.order_mismatch_text {
            Some(t) => t.replace("{which}", which).replace("{tell}", tell),
            None => format!("Your plugins don't match the server's order, so the game would refuse to connect: {which}. {tell}"),
        }
    }
}

/// Fetches this Play's rules. Server copy (checked against the pinned key)
/// first and cached; the cached copy when the server can't be reached or its
/// file can't be trusted; the built-in defaults when there is neither or the
/// server publishes no file.
pub async fn load(client: &reqwest::Client, base_url: &str, data_dir: Option<&Path>, trust: &Trust) -> (Rules, Source, Option<String>) {
    let cache = data_dir.map(|d| d.join(CACHE_FILE));
    let from_cache = |note: String| match cache.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| Rules::parse(&b).ok()) {
        Some(r) => (r, Source::Cache, Some(note)),
        None => (Rules::default(), Source::BuiltIn, Some(note)),
    };
    match trust.fetch(client, base_url, Feed::Rules, Some(Duration::from_secs(8))).await {
        Ok(bytes) => match Rules::parse(&bytes) {
            Ok(r) => {
                if let Some(p) = &cache {
                    let _ = crate::atomicfile::write(p, &bytes);
                }
                (r, Source::Server, None)
            }
            Err(e) => from_cache(format!("the server's rules file was refused ({e})")),
        },
        Err(crate::Error::NotPublished) => {
            // Withdrawn on purpose: the old cached copy must not keep applying.
            if let Some(p) = &cache {
                let _ = std::fs::remove_file(p);
            }
            (Rules::default(), Source::BuiltIn, None)
        }
        Err(e) => from_cache(format!("couldn't get the server's rules file ({e})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::Check;

    fn order_report(items: &[&str]) -> Report {
        let c = Check { id: "serverorder", title: "Server plugin order", status: Status::Fail, detail: "x".into(), items: items.iter().map(|s| s.to_string()).collect() };
        Report { checks: vec![c], worst: Status::Fail }
    }

    #[test]
    fn empty_or_unknown_files_mean_todays_behaviour() {
        let r = Rules::parse(b"{}").unwrap();
        assert_eq!(Rules { rev: "unnamed".into(), ..Rules::default() }, r);
        assert!(Rules::parse(b"not json").is_err());
        assert!(Rules::parse(br#"{"schema":2}"#).is_err());
        // Wrong types keep the default.
        let r = Rules::parse(br#"{"checks":{"udpProbe":"no","serverOrder":5},"order":{"emptyCcc":0}}"#).unwrap();
        assert!(r.flags.udp_probe && r.empty_ccc && r.server_order == Level::Fail);
    }

    #[test]
    fn switches_levels_and_lists_are_read() {
        let r = Rules::parse(br#"{"rev":"r1","checks":{"udpProbe":false,"serverOrder":"warn"},"order":{"emptyCcc":false,"rewriteLoadorderTxt":false,"ignore":["Foo.esp","../bad.esp","x.txt"]},
            "allow":{"files":["Data/Foo.esp","../x","C:/x","a\\b","/abs"]},"text":{"orderMismatch":"Order: {which} {tell}"}}"#).unwrap();
        assert_eq!(r.rev, "r1");
        assert!(!r.flags.udp_probe && r.flags.firewall_fix);
        assert_eq!(r.server_order, Level::Warn);
        assert!(!r.empty_ccc && !r.rewrite_loadorder);
        assert_eq!(r.order_ignore, vec!["Foo.esp".to_string()]);
        assert_eq!(r.allow_files, vec!["Data/Foo.esp".to_string()]);
        assert_eq!(r.order_mismatch_message("a", "b"), "Order: a b");
    }

    #[test]
    fn the_kill_switch_ignores_everything_else() {
        let r = Rules::parse(br#"{"rev":"r2","off":true,"checks":{"udpProbe":false},"order":{"emptyCcc":false}}"#).unwrap();
        assert_eq!(r, Rules { rev: "r2".into(), ..Rules::default() });
    }

    #[test]
    fn text_is_plain_and_short() {
        let long = "x".repeat(401);
        let r = Rules::parse(format!(r#"{{"text":{{"orderMismatch":"{long}"}}}}"#).as_bytes()).unwrap();
        assert!(r.order_mismatch_text.is_none());
        let r = Rules::parse(br#"{"text":{"orderMismatch":"a\u0007b"}}"#).unwrap();
        assert_eq!(r.order_mismatch_text.as_deref(), Some("ab"));
        let d = Rules::default();
        assert!(d.order_mismatch_message("p", "t").starts_with("Your plugins don't match the server's order"));
    }

    #[test]
    fn status_json_and_rules_both_have_to_allow_a_part() {
        let r = Rules::parse(br#"{"checks":{"udpProbe":false}}"#).unwrap();
        let status = Flags { firewall_fix: false, ..Flags::default() };
        let f = r.narrow(status);
        assert!(!f.firewall_fix && !f.udp_probe && f.connect_watch);
    }

    #[test]
    fn ignored_plugins_and_levels_change_what_stops_play() {
        let items = ["position 5: the server loads AetherialDawn.esp, this PC loads ccASVSSE001-ALMSIVI.esm", "Other.esp: 1 bytes, the server's is 2"];
        // Default: unchanged, still a failure.
        let mut rep = order_report(&items);
        Rules::default().adjust_order_check(&mut rep);
        assert_eq!(rep.worst, Status::Fail);
        // Ignoring one plugin keeps the other finding.
        let r = Rules { order_ignore: vec!["ccASVSSE001-ALMSIVI.esm".into()], ..Rules::default() };
        let mut rep = order_report(&items);
        r.adjust_order_check(&mut rep);
        assert_eq!((rep.worst, rep.checks[0].items.len()), (Status::Fail, 1));
        // Ignoring both clears it.
        let r = Rules { order_ignore: vec!["ccASVSSE001-ALMSIVI.esm".into(), "other.esp".into()], ..Rules::default() };
        let mut rep = order_report(&items);
        r.adjust_order_check(&mut rep);
        assert_eq!(rep.worst, Status::Info);
        // Levels.
        for (lvl, want) in [(Level::Warn, Status::Warn), (Level::Off, Status::Info)] {
            let mut rep = order_report(&items);
            Rules { server_order: lvl, ..Rules::default() }.adjust_order_check(&mut rep);
            assert_eq!(rep.worst, want);
        }
    }

    #[test]
    fn the_shipped_server_file_is_todays_behaviour() {
        let shipped = include_bytes!("../../server/launcher-rules.json");
        let r = Rules::parse(shipped).unwrap();
        assert_eq!(Rules { rev: r.rev.clone(), ..Rules::default() }, r);
        assert_ne!(r.rev, "unnamed");
    }

    #[tokio::test]
    async fn an_unreachable_server_uses_the_saved_copy_then_the_defaults() {
        let dir = std::env::temp_dir().join(format!("ad-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let trust = Trust::new(crate::feedsig::pinned_keys(), false, None);
        let client = reqwest::Client::new();
        // Nothing saved, nothing reachable (port 1): built-in.
        let (r, src, note) = load(&client, "http://127.0.0.1:1", Some(&dir), &trust).await;
        assert_eq!((src, r), (Source::BuiltIn, Rules::default()));
        assert!(note.is_some());
        // A saved copy is used.
        std::fs::write(dir.join(CACHE_FILE), br#"{"rev":"saved","checks":{"udpProbe":false}}"#).unwrap();
        let (r, src, _) = load(&client, "http://127.0.0.1:1", Some(&dir), &trust).await;
        assert_eq!((src, r.rev.as_str(), r.flags.udp_probe), (Source::Cache, "saved", false));
        // A saved copy that no longer parses is not used.
        std::fs::write(dir.join(CACHE_FILE), b"junk").unwrap();
        assert_eq!(load(&client, "http://127.0.0.1:1", Some(&dir), &trust).await.1, Source::BuiltIn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
