//! Changes the show file while BlinderKitten has it open.
//!
//! BK keeps the show in memory, so editing the `.olga` alone would be lost on
//! its next save. Via its OSC Remote Control (Preferences → OSC Remote Control,
//! UDP 42000) we first make BK save (`/saveFile`), then edit the file and make
//! BK load it again (`/openFile <path>`).
//!
//! Nothing may get lost on the way: Tapetum waits until the saved file is
//! complete, gives up without touching anything if BK's show changed after
//! saving (someone operated it, a cue ran), and checks that BK has the edited
//! file loaded afterwards.

use crate::i18n::tr;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};
use serde_json::Value;

/// What BK's remote control reports about itself (`GET /?HOST_INFO`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInfo {
    pub version: String,
    /// Show file open in BK; empty for an unsaved new show.
    pub file_path: PathBuf,
    /// BK has changes that aren't saved yet.
    pub unsaved: bool,
    /// When the show file was last written and when the show in BK last
    /// changed (Unix milliseconds), if BK reports them.
    pub file_time: Option<i64>,
    pub session_time: Option<i64>,
}

/// BK notes a change a millisecond after its own save; more than this counts.
const CHANGE_TOLERANCE_MS: i64 = 500;

impl HostInfo {
    pub fn has_open(&self, show: &Path) -> bool {
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        !self.file_path.as_os_str().is_empty() && canon(&self.file_path) == canon(show)
    }
}

/// Asks BK's remote control (HTTP on the same port) which show it has open.
/// `None` if BK isn't running or its OSC Remote Control is disabled.
pub fn host_info(port: u16) -> Option<HostInfo> {
    parse_host_info(&http_get(port, "/?HOST_INFO")?)
}

/// JSON of a container (OSCQuery format) from BK's remote control, e.g. `/group`.
pub fn remote_json(port: u16, path: &str) -> Option<Value> {
    let response = http_get(port, path)?;
    serde_json::from_slice(split_body(&response)?.1).ok()
}

fn http_get(port: u16, path: &str) -> Option<Vec<u8>> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes()).ok()?;
    let mut response = Vec::new();
    // The server may keep the connection open; a timeout after the body is fine.
    let mut buf = [0u8; 16384];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                response.extend_from_slice(&buf[..n]);
                // BK's answers are small; anything else isn't BK.
                if response.len() > 64 << 20 {
                    return None;
                }
                if body_complete(&response) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    Some(response)
}

fn split_body(response: &[u8]) -> Option<(&[u8], &[u8])> {
    let i = response.windows(4).position(|w| w == b"\r\n\r\n")?;
    Some((&response[..i], &response[i + 4..]))
}

fn body_complete(response: &[u8]) -> bool {
    let Some((head, body)) = split_body(response) else { return false };
    let head = String::from_utf8_lossy(head).to_ascii_lowercase();
    let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok());
    len.is_some_and(|len| body.len() >= len)
}

fn parse_host_info(response: &[u8]) -> Option<HostInfo> {
    let (_, body) = split_body(response)?;
    let v: Value = serde_json::from_slice(body).ok()?;
    let meta = v.get("METADATA")?;
    let s = |k: &str| meta.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (file_time, session_time) = (parse_time(&s("fileModificationTime")), parse_time(&s("sessionModificationTime")));
    let unsaved = match (file_time, session_time) {
        (Some(f), Some(t)) => t - f > CHANGE_TOLERANCE_MS,
        _ => s("sessionModificationTime") > s("fileModificationTime"),
    };
    Some(HostInfo { version: s("version"), file_path: PathBuf::from(s("filePath")), unsaved, file_time, session_time })
}

/// `2026-10-24T12:20:00.580+02:00` as Unix milliseconds.
fn parse_time(s: &str) -> Option<i64> {
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, sec) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    let rest = s.get(19..)?;
    let (ms, zone) = match rest.strip_prefix('.') {
        Some(r) => {
            let digits = r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len());
            let frac = &r[..digits];
            let ms = format!("{frac:0<3}").get(..3)?.parse::<i64>().ok()?;
            (ms, &r[digits..])
        }
        None => (0, rest),
    };
    let offset_min = match zone.as_bytes().first() {
        None | Some(b'Z') => 0,
        Some(&sign) => {
            let z = zone.get(1..)?;
            let (zh, zm) = (z.get(0..2)?.parse::<i64>().ok()?, z.get(3..5).unwrap_or("00").parse::<i64>().ok()?);
            (zh * 60 + zm) * if sign == b'-' { -1 } else { 1 }
        }
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + sec - offset_min * 60) * 1000) + ms)
}

fn unix_ms(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    AddMirrors { listen_port: u16 },
    RemoveMirrors,
    /// Tapetum's 2D plans in BK from these fixtures (ID, position).
    Plans { fixtures: Vec<(i64, [f32; 3])> },
    RemovePlans,
}

impl Edit {
    /// What the edit does, for the notice before BK reloads the show.
    pub fn describe(&self) -> &'static str {
        match self {
            Edit::AddMirrors { .. } => tr("Tapetum legt die Tapetum-Interfaces in der Show an oder bringt sie auf den neuesten Stand.", "Tapetum adds the Tapetum interfaces to the show or brings them up to date."),
            Edit::RemoveMirrors => tr("Tapetum entfernt die Tapetum-Interfaces aus der Show.", "Tapetum removes the Tapetum interfaces from the show."),
            Edit::Plans { .. } => tr(
                "Tapetum legt die 2D-Pläne „Tapetum Front“, „Tapetum Top“ und „Tapetum Side“ aus den Lampenpositionen an oder bringt sie auf den neuesten Stand. Deine eigenen Pläne bleiben, wie sie sind.",
                "Tapetum lays out the 2D plans “Tapetum Front”, “Tapetum Top” and “Tapetum Side” from the fixture positions, or brings them up to date. Your own plans stay as they are.",
            ),
            Edit::RemovePlans => tr("Tapetum entfernt seine 2D-Pläne aus der Show.", "Tapetum removes its 2D plans from the show."),
        }
    }
}

#[derive(Debug)]
pub enum Progress {
    Step(String),
    Done(String),
    /// BK did not save; the edit is only safe if BK isn't running.
    NoAnswer,
    Failed(String),
}

/// Runs `edit` on a background thread. With `offline`, the file is changed
/// without talking to BK.
pub fn spawn(show: PathBuf, remote_port: u16, edit: Edit, offline: bool) -> mpsc::Receiver<Progress> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let report = |p| {
            let _ = tx.send(p);
        };
        match run(&show, remote_port, edit, offline, &report) {
            Ok(Some(done)) => report(Progress::Done(done)),
            Ok(None) => report(Progress::NoAnswer),
            Err(e) => report(Progress::Failed(format!("{e:#}"))),
        }
    });
    rx
}

/// `Ok(None)` if BK didn't answer the save request.
pub fn run(show: &Path, remote_port: u16, edit: Edit, offline: bool, report: &dyn Fn(Progress)) -> Result<Option<String>> {
    let show = std::fs::canonicalize(show).with_context(|| trf!("{} nicht gefunden", "{} not found", show.display()))?;
    // When BK's show last changed, right after its save.
    let mut saved_at = None;
    if !offline {
        match host_info(remote_port) {
            None => return Ok(None),
            Some(info) if !info.has_open(&show) => bail!("{}", other_show(&info, &show)),
            Some(_) => {}
        }
        // The file as it was: BK saving while it still loads would write a gutted show.
        let presave = std::fs::read(&show)?;
        if let Ok(on_disk) = serde_json::from_slice::<Value>(&presave) {
            let start = Instant::now();
            while still_loading(remote_port, &on_disk) {
                if start.elapsed() > Duration::from_secs(20) {
                    bail!("{}", tr("BlinderKitten lädt die Show noch (oder es fehlt dort etwas, das in der Datei steht). Es wurde nichts geändert – bitte warten, bis BlinderKitten fertig ist, und noch einmal versuchen.", "BlinderKitten is still loading the show (or lacks something the file has). Nothing was changed – please wait until BlinderKitten is done and try again."));
                }
                report(Progress::Step(tr("BlinderKitten lädt die Show noch …", "BlinderKitten is still loading the show …").into()));
                std::thread::sleep(Duration::from_millis(300));
            }
        }
        report(Progress::Step(tr("BlinderKitten speichert …", "BlinderKitten is saving …").into()));
        let before = modified(&show)?;
        send_osc(remote_port, "/saveFile", None)?;
        if !wait_for_change(&show, before, Duration::from_secs(4)) {
            // BK answers but didn't save (still loading, a dialog open …): the
            // file must not be changed behind its back.
            bail!("{}", tr("BlinderKitten hat die Show nicht gespeichert (lädt es gerade oder ist ein Dialog offen?). Es wurde nichts geändert – bitte gleich noch einmal versuchen.", "BlinderKitten didn't save the show (is it loading, or a dialog open?). Nothing was changed – please try again in a moment."));
        }
        let checked = wait_until_complete(&show, Duration::from_secs(6)).and_then(|()| {
            let at = wait_saved(remote_port, &show, Duration::from_secs(3))?;
            let saved: Value = serde_json::from_slice(&std::fs::read(&show)?)?;
            if !matches_bk(remote_port, &saved) {
                bail!(
                    "{}",
                    tr(
                        "BlinderKitten hat die Show unvollständig gespeichert (lädt es gerade noch?). Die Datei ist wiederhergestellt, es wurde nichts geändert – bitte gleich noch einmal versuchen.",
                        "BlinderKitten saved the show incompletely (is it still loading?). The file is restored, nothing was changed – please try again in a moment.",
                    )
                );
            }
            Ok(at)
        });
        match checked {
            Ok(at) => saved_at = Some(at),
            Err(e) => {
                // An incomplete save (BK still loading) goes back to the file as it was.
                let complete = std::fs::read(&show).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).is_some_and(|d| matches_bk(remote_port, &d));
                if !complete {
                    write_atomic_bytes(&show, &presave)?;
                }
                return Err(e);
            }
        }
    }

    report(Progress::Step(tr("Show-Datei wird angepasst …", "Adjusting the show file …").into()));
    let text = std::fs::read_to_string(&show)?;
    let mut doc: Value = serde_json::from_str(&text).context(tr("Show-Datei ist kein gültiges JSON", "The show file isn't valid JSON"))?;
    let (summary, expect) = apply(&edit, &mut doc)?;

    // Nothing changed in BK since its save? Otherwise the reload would lose it.
    if let Some(at) = saved_at {
        unchanged_since(remote_port, &show, at)?;
    }
    let backup = show.with_extension("olga.bak");
    std::fs::write(&backup, &text).with_context(|| trf!("Sicherung {} fehlgeschlagen", "Backup {} failed", backup.display()))?;
    write_atomic(&show, &serde_json::to_string_pretty(&doc)?)?;

    let Some(at) = saved_at else {
        return Ok(Some(trf!("{summary}. Beim nächsten Öffnen in BlinderKitten aktiv.", "{summary}. Active the next time BlinderKitten opens it.", summary = summary)));
    };
    if let Err(e) = unchanged_since(remote_port, &show, at) {
        // Changed in BK while the file was written: put BK's own file back, reload nothing.
        write_atomic(&show, &text)?;
        return Err(e);
    }
    report(Progress::Step(tr("BlinderKitten lädt die Show neu …", "BlinderKitten reloads the show …").into()));
    let written = unix_ms(modified(&show)?);
    send_osc(remote_port, "/openFile", Some(&show.to_string_lossy()))?;
    wait_reloaded(remote_port, &show, written, &doc, Duration::from_secs(30))?;
    let missing = |expect: &[(String, bool)]| -> Vec<String> {
        expect.iter().filter(|(path, present)| remote_json(remote_port, path).is_some_and(|v| !v.is_null()) != *present).map(|(path, _)| path.clone()).collect()
    };
    let start = Instant::now();
    let mut missing_now = missing(&expect);
    while !missing_now.is_empty() && start.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(200));
        missing_now = missing(&expect);
    }
    if !missing_now.is_empty() {
        bail!(
            "{}",
            trf!(
                "BlinderKitten hat die Show neu geladen, aber die Änderung ist dort nicht zu sehen ({}). Die Show vorher liegt als {} daneben.",
                "BlinderKitten reloaded the show, but the change doesn't show there ({}). The show as it was is next to it as {}.",
                missing_now.join(", "),
                backup.display()
            )
        );
    }
    Ok(Some(trf!("{summary}. BlinderKitten hat die Show neu geladen.", "{summary}. BlinderKitten has reloaded the show.", summary = summary)))
}

/// Applies an edit to the show's JSON: a summary, and the containers BK must
/// (or must no longer) have once it reloaded the file.
fn apply(edit: &Edit, doc: &mut Value) -> Result<(String, Vec<(String, bool)>)> {
    let short = |name: &str| bk_olga::short_name::to_short_name(name, false);
    Ok(match edit {
        Edit::AddMirrors { listen_port } => {
            let entries = bk_olga::mirror::add_mirrors(doc, *listen_port)?;
            let list: Vec<String> = entries
                .iter()
                .map(|e| format!("{} » {} ({})", e.original, e.mirror, crate::routing::format_port_address(e.port_address)))
                .collect();
            let expect = entries.iter().map(|e| (format!("/interfaces/{}", short(&e.mirror)), true)).collect();
            (trf!("Tapetum-Interfaces eingerichtet: {}", "Tapetum interfaces set up: {}", list.join(", ")), expect)
        }
        Edit::RemoveMirrors => {
            let removed = bk_olga::mirror::remove_mirrors(doc);
            if removed.is_empty() {
                bail!("{}", trf!("keine Tapetum-Interfaces vorhanden", "no Tapetum interfaces present"));
            }
            let expect = removed.iter().map(|n| (format!("/interfaces/{}", short(n)), false)).collect();
            (trf!("Entfernt: {}", "Removed: {}", removed.join(", ")), expect)
        }
        Edit::Plans { fixtures } => {
            let plans = bk_olga::plans::set_plans(doc, fixtures).map_err(anyhow::Error::msg)?;
            let names: Vec<&str> = plans.iter().map(|(_, n)| n.as_str()).collect();
            let expect = plans.iter().map(|(id, n)| (bk_olga::plans::remote_path(*id, n), true)).collect();
            (trf!("2D-Pläne angelegt: {} ({} Lampen)", "2D plans laid out: {} ({} fixtures)", names.join(", "), fixtures.len()), expect)
        }
        Edit::RemovePlans => {
            let removed = bk_olga::plans::remove_plans(doc);
            if removed.is_empty() {
                bail!("{}", tr("keine 2D-Pläne von Tapetum vorhanden", "no 2D plans of Tapetum present"));
            }
            let expect = removed.iter().map(|(id, n)| (bk_olga::plans::remote_path(*id, n), false)).collect();
            (trf!("Entfernt: {}", "Removed: {}", removed.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(", ")), expect)
        }
    })
}

fn other_show(info: &HostInfo, show: &Path) -> String {
    let open = if info.file_path.as_os_str().is_empty() { tr("eine ungespeicherte Show", "an unsaved show").into() } else { info.file_path.display().to_string() };
    trf!("In BlinderKitten ist {open} geöffnet, nicht {}. Bitte dort diese Show öffnen.", "BlinderKitten has {open} open, not {}. Please open this show there.", show.display(), open = open)
}

/// Waits until a file BK is writing is complete: the same size and time twice
/// in a row, and valid JSON.
fn wait_until_complete(path: &Path, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    let mut last = None;
    while start.elapsed() < timeout {
        let meta = std::fs::metadata(path)?;
        let now = (meta.len(), meta.modified()?);
        if last == Some(now) && std::fs::read(path).is_ok_and(|b| serde_json::from_slice::<serde::de::IgnoredAny>(&b).is_ok()) {
            return Ok(());
        }
        last = Some(now);
        std::thread::sleep(Duration::from_millis(150));
    }
    bail!("{}", tr("BlinderKitten hat die Show nicht vollständig gespeichert. Es wurde nichts geändert.", "BlinderKitten didn't finish saving the show. Nothing was changed."))
}

/// Waits until BK reports the show as saved; returns when it last changed.
fn wait_saved(port: u16, show: &Path, timeout: Duration) -> Result<i64> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(info) = host_info(port) {
            if !info.has_open(show) {
                bail!("{}", other_show(&info, show));
            }
            if !info.unsaved {
                return Ok(info.session_time.or(info.file_time).unwrap_or(0));
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("{}", busy_message())
}

/// Fails if BK's show changed (or another one is open) since `at`.
fn unchanged_since(port: u16, show: &Path, at: i64) -> Result<()> {
    let info = host_info(port).ok_or_else(|| anyhow::anyhow!("{}", tr("BlinderKitten antwortet nicht mehr. Es wurde nichts geändert.", "BlinderKitten no longer answers. Nothing was changed.")))?;
    if !info.has_open(show) {
        bail!("{}", other_show(&info, show));
    }
    if info.session_time.is_some_and(|t| t - at > CHANGE_TOLERANCE_MS) {
        bail!("{}", busy_message());
    }
    Ok(())
}

fn busy_message() -> String {
    tr(
        "In BlinderKitten hat sich gerade etwas geändert (bedient, eine Cue oder ein Effekt läuft). Es wurde nichts geändert – bitte BlinderKitten kurz in Ruhe lassen und noch einmal versuchen.",
        "Something just changed in BlinderKitten (operated, a cue or an effect running). Nothing was changed – please leave BlinderKitten alone for a moment and try again.",
    )
    .into()
}

/// Items of a top-level manager in a show file (`fixture`, `interfaces`).
fn items(doc: &Value, key: &str) -> usize {
    doc.get(key).and_then(|m| m.get("items")).and_then(Value::as_array).map_or(0, Vec::len)
}

/// Items BK has of a manager right now; `None` if it doesn't say.
fn bk_items(port: u16, path: &str) -> Option<usize> {
    remote_json(port, path)?.get("CONTENTS").and_then(Value::as_object).map(serde_json::Map::len)
}

/// Managers BK reports item by item over its remote control.
const MANAGERS: [&str; 12] = ["interfaces", "fixtureType", "channelFamilies", "group", "fixture", "preset", "cuelist", "effect", "carousel", "mapper", "2dPlans", "tracker"];

/// What BK holds of a show file once it is loaded: (remote path, items), with
/// the cues of every cuelist (BK loads them last).
fn inventory(doc: &Value) -> Vec<(String, usize)> {
    let mut list: Vec<(String, usize)> = MANAGERS.iter().filter(|k| doc.get(**k).is_some()).map(|k| (format!("/{k}"), items(doc, k))).collect();
    for cl in doc.get("cuelist").and_then(|m| m.get("items")).and_then(Value::as_array).into_iter().flatten() {
        if let Some(name) = cl.get("niceName").and_then(Value::as_str) {
            let cues = cl.get("containers").and_then(|c| c.get("cues")).and_then(|c| c.get("items")).and_then(Value::as_array).map_or(0, Vec::len);
            list.push((format!("/cuelist/{}/cues", bk_olga::short_name::to_short_name(name, false)), cues));
        }
    }
    list
}

/// BK holds exactly what the file has: fully loaded, and a save of it complete.
/// Counts BK doesn't report don't count against it.
fn matches_bk(port: u16, doc: &Value) -> bool {
    inventory(doc).iter().all(|(path, n)| bk_items(port, path).is_none_or(|m| m == *n))
}

/// BK holds less of something than the file: it is still loading it (saving
/// now would write a gutted show).
fn still_loading(port: u16, doc: &Value) -> bool {
    inventory(doc).iter().any(|(path, n)| bk_items(port, path).is_some_and(|m| m < *n))
}

/// Waits until BK has loaded the file written at `written` (Unix ms). BK
/// empties the show at once and fills it bit by bit; it is done when it
/// reports this file's time, the show as unchanged since then (the same time
/// to the millisecond) and as many fixtures and interfaces as the file.
fn wait_reloaded(port: u16, show: &Path, written: i64, doc: &Value, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(info) = host_info(port)
            && info.has_open(show)
            && let (Some(file), Some(session)) = (info.file_time, info.session_time)
            && (file - written).abs() <= 2
            && (session - file).abs() <= 5
            && matches_bk(port, doc)
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    bail!(
        "{}",
        trf!(
            "BlinderKitten hat die geänderte Show nicht geladen. Fragt es dort, ob gespeichert werden soll, „Save“ wählen und es hier noch einmal versuchen; sonst die Show in BlinderKitten neu öffnen. Die Show vorher liegt als {} daneben.",
            "BlinderKitten didn't load the changed show. If it asks there whether to save, choose “Save” and try again here; otherwise open the show in BlinderKitten again. The show as it was is next to it as {}.",
            show.with_extension("olga.bak").display()
        )
    )
}

fn modified(path: &Path) -> Result<SystemTime> {
    Ok(std::fs::metadata(path)?.modified()?)
}

fn wait_for_change(path: &Path, before: SystemTime, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if modified(path).is_ok_and(|m| m != before) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn write_atomic(path: &Path, content: &str) -> Result<()> {
    write_atomic_bytes(path, content.as_bytes())
}

fn write_atomic_bytes(path: &Path, content: &[u8]) -> Result<()> {
    let tmp = path.with_extension("olga.tapetum.tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn send_osc(port: u16, address: &str, arg: Option<&str>) -> Result<()> {
    let args: Vec<OscArg> = arg.map(OscArg::Str).into_iter().collect();
    send_osc_args(port, address, &args)
}

pub fn send_osc_args(port: u16, address: &str, args: &[OscArg]) -> Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
    socket.send_to(&osc_message(address, args), (Ipv4Addr::LOCALHOST, port))?;
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub enum OscArg<'a> {
    Str(&'a str),
    Float(f32),
}

/// Encodes an OSC message.
pub fn osc_message(address: &str, args: &[OscArg]) -> Vec<u8> {
    fn push_str(buf: &mut Vec<u8>, s: &str) {
        buf.extend_from_slice(s.as_bytes());
        // At least one NUL, padded to a multiple of four.
        buf.extend(std::iter::repeat_n(0, 4 - s.len() % 4));
    }
    let mut buf = Vec::new();
    push_str(&mut buf, address);
    let tags: String = std::iter::once(',').chain(args.iter().map(|a| if matches!(a, OscArg::Str(_)) { 's' } else { 'f' })).collect();
    push_str(&mut buf, &tags);
    for a in args {
        match a {
            OscArg::Str(s) => push_str(&mut buf, s),
            OscArg::Float(f) => buf.extend_from_slice(&f.to_be_bytes()),
        }
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_encoding() {
        assert_eq!(osc_message("/saveFile", &[]), b"/saveFile\0\0\0,\0\0\0");
        assert_eq!(osc_message("/openFile", &[OscArg::Str("/a.olga")]), b"/openFile\0\0\0,s\0\0/a.olga\0");
        assert_eq!(osc_message("/r", &[OscArg::Float(1.0), OscArg::Float(-2.0)]), b"/r\0\0,ff\0\x3f\x80\0\0\xc0\0\0\0");
    }

    #[test]
    fn offline_edit_adds_and_removes() {
        let dir = std::env::temp_dir().join(format!("tapetum-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let show = dir.join("show.olga");
        std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/../bk-olga/tests/show.olga"), &show).unwrap();

        let msg = run(&show, 1, Edit::AddMirrors { listen_port: 6454 }, true, &|_| {}).unwrap().unwrap();
        assert!(msg.contains("DMX1 » Tapetum DMX1 (0:0:0)"), "{msg}");
        let s = bk_olga::load(&show).unwrap();
        assert!(s.interfaces.iter().any(|i| i.name == "Tapetum DMX2"));
        assert!(dir.join("show.olga.bak").exists());

        run(&show, 1, Edit::RemoveMirrors, true, &|_| {}).unwrap().unwrap();
        assert!(!bk_olga::load(&show).unwrap().interfaces.iter().any(|i| i.name.starts_with("Tapetum")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn host_info_response() {
        let body = r#"{"NAME":"BlinderKitten - show","METADATA":{"version":"1.0.1b98","filePath":"/x/show.olga",
            "fileModificationTime":"2026-10-21T18:59:00.000+02:00","sessionModificationTime":"2026-10-21T19:05:00.000+02:00"}}"#;
        let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        assert!(body_complete(response.as_bytes()));
        let info = parse_host_info(response.as_bytes()).unwrap();
        assert_eq!(
            info,
            HostInfo { version: "1.0.1b98".into(), file_path: "/x/show.olga".into(), unsaved: true, file_time: Some(1_792_601_940_000), session_time: Some(1_792_602_300_000) }
        );
    }

    #[test]
    fn inventory_of_the_example() {
        let doc: Value = serde_json::from_slice(include_bytes!("../assets/example/de/Tapetum-Demo.olga")).unwrap();
        let list = inventory(&doc);
        assert!(list.contains(&("/fixture".to_string(), 19)), "{list:?}");
        assert!(list.contains(&("/cuelist/1_Main/cues".to_string(), 8)), "{list:?}");
    }

    #[test]
    fn times_and_the_millisecond_after_saving() {
        assert_eq!(parse_time("2026-10-24T12:20:00.580+02:00"), Some(1_792_837_200_580));
        assert_eq!(parse_time("1999-12-31T23:59:59.999-05:30"), Some(946_704_599_999));
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time(""), None);
        // BK right after its own save: the session is a millisecond younger than the file.
        let body = r#"{"METADATA":{"filePath":"/x/show.olga","fileModificationTime":"2026-10-24T12:20:00.580+02:00","sessionModificationTime":"2026-10-24T12:20:00.581+02:00"}}"#;
        let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        assert!(!parse_host_info(response.as_bytes()).unwrap().unsaved);
    }

    #[test]
    fn no_answer_without_bk() {
        let dir = std::env::temp_dir().join(format!("tapetum-noanswer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let show = dir.join("show.olga");
        std::fs::write(&show, "{}").unwrap();
        // Nothing listens on port 9; the file must stay untouched.
        assert!(run(&show, 9, Edit::RemoveMirrors, false, &|_| {}).unwrap().is_none());
        assert_eq!(std::fs::read_to_string(&show).unwrap(), "{}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
