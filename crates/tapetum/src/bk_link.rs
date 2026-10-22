//! Changes the show file while BlinderKitten has it open.
//!
//! BK keeps the show in memory, so editing the `.olga` alone would be lost on
//! its next save. Via its OSC Remote Control (Preferences → OSC Remote Control,
//! UDP 42000) we first make BK save (`/saveFile`), then edit the file and make
//! BK load it again (`/openFile <path>`).

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
}

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
    Some(HostInfo {
        version: s("version"),
        file_path: PathBuf::from(s("filePath")),
        unsaved: s("sessionModificationTime") > s("fileModificationTime"),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    AddMirrors { listen_port: u16 },
    RemoveMirrors,
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
    if !offline {
        match host_info(remote_port) {
            None => return Ok(None),
            Some(info) if !info.has_open(&show) => {
                let open = if info.file_path.as_os_str().is_empty() { tr("eine ungespeicherte Show", "an unsaved show").into() } else { info.file_path.display().to_string() };
                bail!("{}", trf!("In BlinderKitten ist {open} geöffnet, nicht {}. Bitte dort diese Show öffnen.", "BlinderKitten has {open} open, not {}. Please open this show there.", show.display(), open = open));
            }
            Some(_) => {}
        }
        report(Progress::Step(tr("BlinderKitten speichert …", "BlinderKitten is saving …").into()));
        let before = modified(&show)?;
        send_osc(remote_port, "/saveFile", None)?;
        if !wait_for_change(&show, before, Duration::from_secs(4)) {
            return Ok(None);
        }
        // BK writes via a temporary file; give a slow disk a moment anyway.
        std::thread::sleep(Duration::from_millis(300));
    }

    report(Progress::Step(tr("Show-Datei wird angepasst …", "Adjusting the show file …").into()));
    let text = std::fs::read_to_string(&show)?;
    let mut doc: Value = serde_json::from_str(&text).context(tr("Show-Datei ist kein gültiges JSON", "The show file isn't valid JSON"))?;
    let summary = match edit {
        Edit::AddMirrors { listen_port } => {
            let entries = bk_olga::mirror::add_mirrors(&mut doc, listen_port)?;
            let list: Vec<String> = entries
                .iter()
                .map(|e| format!("{} » {} ({})", e.original, e.mirror, crate::routing::format_port_address(e.port_address)))
                .collect();
            trf!("Tapetum-Interfaces eingerichtet: {}", "Tapetum interfaces set up: {}", list.join(", "))
        }
        Edit::RemoveMirrors => {
            let removed = bk_olga::mirror::remove_mirrors(&mut doc);
            if removed.is_empty() {
                bail!("{}", trf!("keine Tapetum-Interfaces vorhanden", "no Tapetum interfaces present"));
            }
            trf!("Entfernt: {}", "Removed: {}", removed.join(", "))
        }
    };

    let backup = show.with_extension("olga.bak");
    std::fs::write(&backup, &text).with_context(|| trf!("Sicherung {} fehlgeschlagen", "Backup {} failed", backup.display()))?;
    write_atomic(&show, &serde_json::to_string_pretty(&doc)?)?;

    if offline {
        return Ok(Some(trf!("{summary}. Beim nächsten Öffnen in BlinderKitten aktiv.", "{summary}. Active the next time BlinderKitten opens it.", summary = summary)));
    }
    report(Progress::Step(tr("BlinderKitten lädt die Show neu …", "BlinderKitten reloads the show …").into()));
    send_osc(remote_port, "/openFile", Some(&show.to_string_lossy()))?;
    Ok(Some(trf!("{summary}. BlinderKitten hat die Show neu geladen.", "{summary}. BlinderKitten has reloaded the show.", summary = summary)))
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
        assert_eq!(info, HostInfo { version: "1.0.1b98".into(), file_path: "/x/show.olga".into(), unsaved: true });
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
