//! Why a player's game starts but never reaches the server: Windows Firewall
//! rules for SkyrimSE.exe, a UDP reachability probe of the game server, and
//! the "game ran but never connected" finding for the session report.
//!
//! Everything here is advisory. Nothing in this module may stop Play: every
//! failure is a log line and a health check, never an error for the player.
//! Staff can switch each part off in status.json (`launcher.firewallFix`,
//! `launcher.udpProbe`, `launcher.connectWatch`, `launcher.platformGuard` set to false).

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::health::{Check, Status};

/// The rule the launcher creates; also how it finds its own rule again.
pub const RULE_NAME: &str = "Aetherial Dawn - Skyrim";

/// A repaired-or-declined firewall fix is not asked for again for this long.
pub const ASK_AGAIN_SECS: u64 = 3 * 24 * 3600;

/// Which parts staff left on. A missing key, or a status file that could not
/// be read, leaves a part ON; only an explicit `false` turns one off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    pub firewall_fix: bool,
    pub udp_probe: bool,
    pub connect_watch: bool,
    /// Set other servers' Skyrim Platform scripts aside for the session.
    pub platform_guard: bool,
}

impl Default for Flags {
    fn default() -> Self {
        Flags { firewall_fix: true, udp_probe: true, connect_watch: true, platform_guard: true }
    }
}

impl Flags {
    pub fn from_status(status: Option<&serde_json::Value>) -> Flags {
        let launcher = status.and_then(|s| s.get("launcher"));
        let on = |key: &str| launcher.and_then(|l| l.get(key)).and_then(|v| v.as_bool()) != Some(false);
        Flags { firewall_fix: on("firewallFix"), udp_probe: on("udpProbe"), connect_watch: on("connectWatch"), platform_guard: on("platformGuard") }
    }
}

// ---------- Windows Firewall ----------

/// One firewall rule that names SkyrimSE.exe, as PowerShell reports it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Rule {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub enabled: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub direction: String,
}

impl Rule {
    fn is_on(&self) -> bool {
        // [string]$rule.Enabled is "True"/"False" (some builds print 1/0).
        matches!(self.enabled.to_ascii_lowercase().as_str(), "true" | "1")
    }
    fn blocks(&self) -> bool {
        self.is_on() && matches!(self.action.to_ascii_lowercase().as_str(), "block" | "2")
    }
    fn allows(&self, direction: &str) -> bool {
        let d = self.direction.to_ascii_lowercase();
        let want = if direction == "in" { ["inbound", "1"] } else { ["outbound", "2"] };
        self.is_on() && matches!(self.action.to_ascii_lowercase().as_str(), "allow" | "1") && want.contains(&d.as_str())
    }
}

/// Reads the JSON `ConvertTo-Json` prints: nothing, one object, or an array.
pub fn parse_rules(out: &str) -> Vec<Rule> {
    let t = out.trim().trim_start_matches('\u{feff}');
    if t.is_empty() {
        return vec![];
    }
    match serde_json::from_str::<serde_json::Value>(t) {
        Ok(serde_json::Value::Array(a)) => a.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect(),
        Ok(v @ serde_json::Value::Object(_)) => serde_json::from_value(v).ok().into_iter().collect(),
        _ => vec![],
    }
}

/// What the rules for SkyrimSE.exe add up to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// Names of switched-on Block rules (Windows makes these when the
    /// first-run "allow Skyrim?" prompt is cancelled or never seen).
    pub blockers: Vec<String>,
    pub allow_in: bool,
    pub allow_out: bool,
}

impl Verdict {
    pub fn of(rules: &[Rule]) -> Verdict {
        Verdict {
            blockers: rules.iter().filter(|r| r.blocks()).map(|r| r.name.clone()).collect(),
            allow_in: rules.iter().any(|r| r.allows("in")),
            allow_out: rules.iter().any(|r| r.allows("out")),
        }
    }
    pub fn needs_fix(&self) -> bool {
        !self.blockers.is_empty() || !self.allow_in || !self.allow_out
    }
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A path as Windows writes it, which is how firewall rules store it.
pub fn win_path(p: &Path) -> String {
    p.display().to_string().replace('/', "\\")
}

/// PowerShell that prints the rules naming `program` as JSON. Reading needs no
/// administrator rights.
pub fn read_script(program: &str) -> String {
    format!(
        "$p = {}; Get-NetFirewallApplicationFilter -ErrorAction SilentlyContinue | Where-Object {{ $_.Program -ieq $p }} | ForEach-Object {{ $r = $_ | Get-NetFirewallRule; [pscustomobject]@{{ Name = $r.Name; DisplayName = $r.DisplayName; Enabled = [string]$r.Enabled; Action = [string]$r.Action; Direction = [string]$r.Direction }} }} | ConvertTo-Json -Compress",
        ps_quote(program)
    )
}

/// The PowerShell the elevated helper runs: take away the Block rules, then
/// make (or remake) the launcher's own allow rules for both directions on
/// every network profile.
pub fn fix_script(program: &str, blockers: &[String]) -> String {
    let mut s = String::from("$ErrorActionPreference = 'Stop'; ");
    for name in blockers {
        s.push_str(&format!("Remove-NetFirewallRule -Name {} -ErrorAction SilentlyContinue; ", ps_quote(name)));
    }
    s.push_str(&format!("Remove-NetFirewallRule -DisplayName {} -ErrorAction SilentlyContinue; ", ps_quote(RULE_NAME)));
    for dir in ["Inbound", "Outbound"] {
        s.push_str(&format!(
            "New-NetFirewallRule -DisplayName {} -Program {} -Direction {dir} -Action Allow -Profile Any | Out-Null; ",
            ps_quote(RULE_NAME),
            ps_quote(program)
        ));
    }
    s
}

/// PowerShell's -EncodedCommand: base64 of the script as UTF-16LE, so no
/// quoting survives to be mangled on the way to the elevated process.
pub fn encode_command(script: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// The marker file's text: the game path, when it was tried, how it ended.
pub fn marker_text(program: &str, now_secs: u64, result: &str) -> String {
    format!("{program}|{now_secs}|{result}")
}

/// Whether the elevated fix may be tried now: never tried for this game
/// path, or tried long enough ago (the player may have said no in a hurry).
pub fn may_ask(marker: Option<&str>, program: &str, now_secs: u64) -> bool {
    let Some(m) = marker else { return true };
    let mut parts = m.trim().rsplitn(3, '|');
    let (_result, at, path) = (parts.next(), parts.next().and_then(|s| s.parse::<u64>().ok()), parts.next());
    match (path, at) {
        (Some(p), Some(at)) if p.eq_ignore_ascii_case(program) => now_secs.saturating_sub(at) >= ASK_AGAIN_SECS,
        _ => true,
    }
}

/// How the firewall step ended, for the log and the health report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Firewall {
    /// Rules already let Skyrim through.
    Fine,
    /// The launcher fixed the rules (one Windows permission prompt).
    Repaired(Vec<String>),
    /// Needed a fix, but the player said no, or Windows refused it.
    NotFixed(String),
    /// Needed a fix, but one was already tried recently.
    SkippedRecently,
    /// Could not read the rules (not Windows, PowerShell missing, timeout).
    Unknown(String),
    /// Staff switched this part off.
    Off,
}

pub fn firewall_check(f: &Firewall) -> Check {
    let c = |status, detail: String, items: Vec<String>| Check { id: "firewall", title: "Windows Firewall", status, detail, items };
    match f {
        Firewall::Fine => c(Status::Ok, "Windows Firewall lets Skyrim through.".into(), vec![]),
        Firewall::Repaired(removed) => c(
            Status::Info,
            "The launcher fixed Windows Firewall so Skyrim can reach the server.".into(),
            removed.iter().map(|n| format!("removed blocking rule {n}")).collect(),
        ),
        Firewall::NotFixed(why) => c(
            Status::Warn,
            "Windows Firewall may be stopping Skyrim from reaching the server, and the launcher couldn't fix it. Press Play again and say Yes to the Windows permission prompt.".into(),
            vec![why.clone()],
        ),
        Firewall::SkippedRecently => c(Status::Warn, "Windows Firewall may be stopping Skyrim from reaching the server. The launcher asked recently and will ask again in a few days.".into(), vec![]),
        Firewall::Unknown(why) => c(Status::Info, "Couldn't check Windows Firewall.".into(), vec![why.clone()]),
        Firewall::Off => c(Status::Info, "The Windows Firewall fix is switched off by staff.".into(), vec![]),
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn powershell(args: &[&str]) -> std::io::Result<std::process::Output> {
        Command::new("powershell.exe").creation_flags(CREATE_NO_WINDOW).args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass"]).args(args).output()
    }

    pub fn read_rules(program: &str) -> Result<Vec<Rule>, String> {
        let out = powershell(&["-EncodedCommand", encode_command(&read_script(program)).as_str()]).map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!("PowerShell said: {}", String::from_utf8_lossy(&out.stderr).trim().chars().take(200).collect::<String>()));
        }
        Ok(parse_rules(&String::from_utf8_lossy(&out.stdout)))
    }

    /// Runs the fix in an elevated PowerShell (one Windows permission prompt)
    /// and waits for it. Err when the player declined or Windows refused.
    pub fn run_elevated(script: &str) -> Result<(), String> {
        let launch = format!(
            "Start-Process -FilePath powershell.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand','{}'",
            encode_command(script)
        );
        let out = powershell(&["-EncodedCommand", encode_command(&launch).as_str()]).map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().chars().take(200).collect::<String>())
        }
    }
}

/// Makes sure Windows Firewall lets `exe` through; asks Windows for
/// permission only when something is wrong. `marker` remembers the last
/// attempt. `before_prompt` runs just before the permission prompt shows.
#[cfg(windows)]
pub fn ensure_firewall(exe: &Path, marker: Option<&Path>, now_secs: u64, before_prompt: &dyn Fn()) -> Firewall {
    let program = win_path(exe);
    let rules = match win::read_rules(&program) {
        Ok(r) => r,
        Err(e) => return Firewall::Unknown(e),
    };
    let verdict = Verdict::of(&rules);
    if !verdict.needs_fix() {
        return Firewall::Fine;
    }
    let seen = marker.and_then(|m| std::fs::read_to_string(m).ok());
    // The prompt is Windows' own UAC dialog, so it is not repeated on every
    // Play: once per game path, then again only after a few days.
    if !may_ask(seen.as_deref(), &program, now_secs) {
        return Firewall::SkippedRecently;
    }
    before_prompt();
    let result = win::run_elevated(&fix_script(&program, &verdict.blockers));
    let after = win::read_rules(&program).map(|r| Verdict::of(&r));
    let outcome = match (&result, &after) {
        (Ok(()), Ok(v)) if !v.needs_fix() => Firewall::Repaired(verdict.blockers.clone()),
        (Ok(()), Ok(v)) => Firewall::NotFixed(format!("still needs a fix after the change: {} blocking rule(s), allow in={}, out={}", v.blockers.len(), v.allow_in, v.allow_out)),
        (Ok(()), Err(e)) => Firewall::NotFixed(format!("couldn't read the rules back: {e}")),
        (Err(e), _) => Firewall::NotFixed(if e.is_empty() { "the permission prompt was declined".into() } else { e.clone() }),
    };
    if let Some(m) = marker {
        let tag = match &outcome {
            Firewall::Repaired(_) => "repaired",
            _ => "not-fixed",
        };
        let _ = std::fs::write(m, marker_text(&program, now_secs, tag));
    }
    outcome
}

#[cfg(not(windows))]
pub fn ensure_firewall(_exe: &Path, _marker: Option<&Path>, _now_secs: u64, _before_prompt: &dyn Fn()) -> Firewall {
    Firewall::Unknown("not Windows".into())
}

// ---------- UDP reachability ----------

/// RakNet's unconnected ping: id 0x01, a time, the offline-message magic, a
/// client guid. A RakNet server (SkyMP's) answers it with 0x1C.
pub fn ping_packet(time: u64, guid: u64) -> Vec<u8> {
    let mut p = vec![0x01];
    p.extend_from_slice(&time.to_be_bytes());
    p.extend_from_slice(&[0x00, 0xff, 0xff, 0x00, 0xfe, 0xfe, 0xfe, 0xfe, 0xfd, 0xfd, 0xfd, 0xfd, 0x12, 0x34, 0x56, 0x78]);
    p.extend_from_slice(&guid.to_be_bytes());
    p
}

/// An unconnected pong (0x1C) or the open-connections variant (0x1D).
pub fn is_pong(buf: &[u8]) -> bool {
    matches!(buf.first(), Some(0x1c) | Some(0x1d))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    Answered(u64),
    NoAnswer,
    Unknown(String),
    Off,
}

/// Sends up to `tries` pings, `wait` apart, and waits for a pong.
pub async fn probe_udp(host: &str, port: u16, tries: u32, wait: Duration) -> Probe {
    use tokio::net::UdpSocket;
    let target = match tokio::net::lookup_host((host, port)).await {
        Ok(mut it) => match it.find(|a| a.is_ipv4()) {
            Some(a) => a,
            None => return Probe::Unknown(format!("no IPv4 address for {host}")),
        },
        Err(e) => return Probe::Unknown(format!("couldn't look up {host}: {e}")),
    };
    let sock = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => return Probe::Unknown(format!("couldn't open a UDP socket: {e}")),
    };
    let started = std::time::Instant::now();
    let packet = ping_packet(started.elapsed().as_millis() as u64, rand::random());
    let mut buf = [0u8; 256];
    for _ in 0..tries.max(1) {
        if let Err(e) = sock.send_to(&packet, target).await {
            // A refused or unreachable send is a local network problem.
            return Probe::Unknown(format!("couldn't send to the server: {e}"));
        }
        let deadline = tokio::time::Instant::now() + wait;
        while let Ok(Ok((n, from))) = tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await {
            if from.ip() == target.ip() && is_pong(&buf[..n]) {
                return Probe::Answered(started.elapsed().as_millis() as u64);
            }
        }
    }
    Probe::NoAnswer
}

pub fn udp_check(p: &Probe, port: u16) -> Check {
    let c = |status, detail: String| Check { id: "udp", title: "Game server (UDP)", status, detail, items: vec![] };
    match p {
        Probe::Answered(ms) => c(Status::Ok, format!("The game server answered on UDP {port} in {ms} ms.")),
        Probe::NoAnswer => c(
            Status::Warn,
            format!("The game server did not answer on UDP {port}. A firewall, router, VPN or the network you are on may be blocking it. Try another network, such as a phone hotspot."),
        ),
        Probe::Unknown(why) => c(Status::Info, format!("Couldn't test the game server connection: {why}")),
        Probe::Off => c(Status::Info, "The game server connection test is switched off by staff.".into()),
    }
}

/// Other servers' Skyrim Platform scripts the launcher set aside for this
/// session (they come back when the game closes). Skyrim Platform runs every
/// script in its plugin folders, so a second server's client would run beside
/// ours and could connect elsewhere or fail silently.
pub fn platform_check(set_aside: &[String]) -> Check {
    if set_aside.is_empty() {
        return Check { id: "platform", title: "Other servers' scripts", status: Status::Ok, detail: "No scripts from other servers in Skyrim Platform's folders.".into(), items: vec![] };
    }
    Check {
        id: "platform",
        title: "Other servers' scripts",
        status: Status::Info,
        detail: "Scripts from another server were found in Skyrim Platform's folders. They are set aside while you play on Aetherial Dawn and put back when the game closes.".into(),
        items: set_aside.to_vec(),
    }
}

// ---------- game ran but never connected ----------

/// Skyrim ran, did not crash, got to the main menu, and never loaded a world.
/// When SkyMP connects it loads the player into the world, which SKSE reports
/// as a pre/post load-game or new-game message (ids 2, 3, 7). A log that has
/// "data loaded" (8) but none of those means the client never got a session.
/// Short runs are the player backing out, not a finding.
pub fn never_connected(skse_log: &str, ran_secs: u64) -> bool {
    const MIN_RUN_SECS: u64 = 30;
    if ran_secs < MIN_RUN_SECS {
        return false;
    }
    let sent = |id: u32| skse_log.contains(&format!("dispatch message ({id})"));
    sent(8) && !(sent(2) || sent(3) || sent(7))
}

pub fn connect_check(ran_secs: u64) -> Check {
    Check {
        id: "connect",
        title: "Joined the server",
        status: Status::Warn,
        detail: format!("Skyrim ran for {ran_secs} seconds and never loaded into the world, so it most likely never connected to the server. Common causes: Windows Firewall, a router or VPN blocking UDP, or an overlay."),
        items: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(name: &str, enabled: &str, action: &str, direction: &str) -> Rule {
        Rule { name: name.into(), display_name: name.into(), enabled: enabled.into(), action: action.into(), direction: direction.into() }
    }

    #[test]
    fn flags_only_turn_off_on_an_explicit_false() {
        assert_eq!(Flags::from_status(None), Flags::default());
        assert_eq!(Flags::from_status(Some(&serde_json::json!({}))), Flags::default());
        assert_eq!(Flags::from_status(Some(&serde_json::json!({"launcher": {"firewallFix": "no", "udpProbe": 0}}))), Flags::default());
        let f = Flags::from_status(Some(&serde_json::json!({"launcher": {"firewallFix": false, "connectWatch": false}})));
        assert!(!f.firewall_fix && f.udp_probe && !f.connect_watch && f.platform_guard);
        assert!(!Flags::from_status(Some(&serde_json::json!({"launcher": {"platformGuard": false}}))).platform_guard);
    }

    #[test]
    fn parses_none_one_or_many_rules() {
        assert!(parse_rules("").is_empty());
        assert!(parse_rules("not json").is_empty());
        let one = r#"{"Name":"{A}","DisplayName":"Skyrim","Enabled":"True","Action":"Block","Direction":"Inbound"}"#;
        assert_eq!(parse_rules(one).len(), 1);
        let many = format!("\u{feff}[{one},{one}]");
        assert_eq!(parse_rules(&many).len(), 2);
    }

    #[test]
    fn a_block_rule_or_a_missing_allow_needs_a_fix() {
        let ok = [rule("a", "True", "Allow", "Inbound"), rule("b", "True", "Allow", "Outbound")];
        assert!(!Verdict::of(&ok).needs_fix());
        let blocked = [rule("a", "True", "Allow", "Inbound"), rule("b", "True", "Allow", "Outbound"), rule("{X}", "True", "Block", "Inbound")];
        let v = Verdict::of(&blocked);
        assert!(v.needs_fix());
        assert_eq!(v.blockers, vec!["{X}".to_string()]);
        // A switched-off block rule does nothing.
        let off = [rule("a", "True", "Allow", "Inbound"), rule("b", "True", "Allow", "Outbound"), rule("{X}", "False", "Block", "Inbound")];
        assert!(!Verdict::of(&off).needs_fix());
        // Nothing at all: no allow rule yet.
        assert!(Verdict::of(&[]).needs_fix());
        // Numeric enum spellings.
        assert!(Verdict::of(&[rule("{X}", "1", "2", "1")]).blockers.len() == 1);
    }

    #[test]
    fn scripts_quote_names_and_paths() {
        let s = fix_script(r"C:\Games\it's\SkyrimSE.exe", &["{A}".into(), "o'brien".into()]);
        assert!(s.contains("'C:\\Games\\it''s\\SkyrimSE.exe'"));
        assert!(s.contains("Remove-NetFirewallRule -Name 'o''brien'"));
        assert!(s.contains("-Direction Inbound") && s.contains("-Direction Outbound") && s.contains("-Profile Any"));
        assert!(read_script(r"C:\a b\SkyrimSE.exe").contains("-ieq $p"));
    }

    #[test]
    fn encodes_a_command_like_powershell_wants() {
        // "Hi" as UTF-16LE is 48 00 69 00.
        assert_eq!(encode_command("Hi"), "SABpAA==");
        assert_eq!(encode_command("a"), "YQA=");
        assert_eq!(encode_command(""), "");
    }

    #[test]
    fn asks_again_only_for_a_new_path_or_after_days() {
        let p = r"C:\Skyrim\SkyrimSE.exe";
        assert!(may_ask(None, p, 100));
        let m = marker_text(p, 1000, "not-fixed");
        assert!(!may_ask(Some(&m), p, 1000 + 60));
        assert!(!may_ask(Some(&m), &p.to_lowercase(), 1000 + 60));
        assert!(may_ask(Some(&m), p, 1000 + ASK_AGAIN_SECS));
        assert!(may_ask(Some(&m), r"D:\Other\SkyrimSE.exe", 1001));
        assert!(may_ask(Some("garbage"), p, 5));
    }

    #[test]
    fn the_ping_is_a_raknet_unconnected_ping() {
        let p = ping_packet(1, 2);
        assert_eq!(p.len(), 1 + 8 + 16 + 8);
        assert_eq!(p[0], 0x01);
        assert_eq!(&p[9..13], &[0x00, 0xff, 0xff, 0x00]);
        assert!(is_pong(&[0x1c, 0, 0]) && is_pong(&[0x1d]) && !is_pong(&[0x01]) && !is_pong(&[]));
    }

    #[tokio::test]
    async fn a_pong_counts_and_silence_does_not() {
        let server = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = server.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut b = [0u8; 64];
            if let Ok((_, from)) = server.recv_from(&mut b).await {
                let _ = server.send_to(&[0x1c, 0, 0, 0], from).await;
            }
        });
        assert!(matches!(probe_udp("127.0.0.1", port, 2, Duration::from_millis(500)).await, Probe::Answered(_)));
        let quiet = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let qport = quiet.local_addr().unwrap().port();
        assert_eq!(probe_udp("127.0.0.1", qport, 2, Duration::from_millis(100)).await, Probe::NoAnswer);
    }

    #[test]
    fn never_connected_needs_a_long_clean_run_that_loaded_no_world() {
        let menu = "init complete\ndispatch message (6)\ndispatch message (8)\n";
        assert!(never_connected(menu, 99));
        assert!(!never_connected(menu, 12), "a short run is the player backing out");
        assert!(!never_connected(&format!("{menu}dispatch message (3)\n"), 99), "a world loaded");
        assert!(!never_connected(&format!("{menu}dispatch message (7)\n"), 99));
        assert!(!never_connected("init complete\n", 99), "never reached the main menu: a different problem");
    }

    #[test]
    fn checks_have_stable_ids_and_levels() {
        assert_eq!(firewall_check(&Firewall::Fine).status, Status::Ok);
        assert_eq!(firewall_check(&Firewall::NotFixed("x".into())).status, Status::Warn);
        assert_eq!(firewall_check(&Firewall::Unknown("x".into())).status, Status::Info);
        assert_eq!(udp_check(&Probe::NoAnswer, 7777).id, "udp");
        assert_eq!(udp_check(&Probe::NoAnswer, 7777).status, Status::Warn);
        assert_eq!(udp_check(&Probe::Unknown("x".into()), 7777).status, Status::Info);
        assert_eq!(connect_check(99).id, "connect");
        assert_eq!(platform_check(&[]).status, Status::Ok);
        assert_eq!(platform_check(&["Data/Platform/Plugins/a.js".into()]).status, Status::Info);
    }
}
