#![cfg(feature = "watch")]

use std::time::Duration;

#[test]
fn reloads_on_save() {
    let dir = std::env::temp_dir().join(format!("olga-watch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("show.olga");
    std::fs::write(&path, r#"{"fixture": {"items": []}}"#).unwrap();

    let w = bk_olga::watch::watch(&path, Duration::from_millis(100)).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    // Same pattern as an atomic save: write a temp file, then rename over the show.
    let tmp = dir.join("show.olga.tmp");
    std::fs::write(&tmp, r#"{"fixture": {"items": [{"niceName": "1 - A", "parameters": [{"controlAddress": "/name", "value": "A"}]}]}}"#).unwrap();
    std::fs::rename(&tmp, &path).unwrap();

    // Some backends (FSEvents) also report the initial write, so wait for the newest state.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let show = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let show = w.updates.recv_timeout(left).expect("no reload").unwrap();
        if !show.fixtures.is_empty() {
            break show;
        }
    };
    assert_eq!(show.fixtures[0].name, "A");
    std::fs::remove_dir_all(dir).unwrap();
}
