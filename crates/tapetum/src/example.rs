//! The example that ships with Tapetum: a small theatre as an IngeTrazo file, a
//! BlinderKitten show with fixtures and cues, and its project. It opens on the
//! first start; saved into a folder of the user's choice, its files open in
//! BlinderKitten and IngeTrazo like any others. In German or English, as Tapetum
//! is set when it is opened or saved. Built by `scripts/gen-example.py`.

use std::path::{Path, PathBuf};

pub const NAME: &str = "Tapetum-Demo";

struct Files {
    lang: &'static str,
    olga: &'static [u8],
    igz: &'static [u8],
    project: &'static [u8],
}

const DE: Files = Files {
    lang: "de",
    olga: include_bytes!("../assets/example/de/Tapetum-Demo.olga"),
    igz: include_bytes!("../assets/example/de/Tapetum-Demo.igz"),
    project: include_bytes!("../assets/example/de/Tapetum-Demo.tapetum"),
};
const EN: Files = Files {
    lang: "en",
    olga: include_bytes!("../assets/example/en/Tapetum-Demo.olga"),
    igz: include_bytes!("../assets/example/en/Tapetum-Demo.igz"),
    project: include_bytes!("../assets/example/en/Tapetum-Demo.tapetum"),
};

/// The example in the language Tapetum is set to.
fn current() -> &'static Files {
    if crate::i18n::english() { &EN } else { &DE }
}

fn file(dir: &Path, ext: &str) -> PathBuf {
    dir.join(format!("{NAME}.{ext}"))
}

/// Where the example lives until it is saved somewhere, one folder per language.
fn internal_dir(files: &Files) -> Option<PathBuf> {
    Some(crate::i18n::config_dir()?.join("example").join(files.lang))
}

/// The example's show in Tapetum's own folder, written there first; the
/// project is kept, so changes made while trying it out stay.
pub fn open_internal() -> Option<PathBuf> {
    let files = current();
    let dir = internal_dir(files)?;
    std::fs::create_dir_all(&dir).ok()?;
    for (ext, bytes) in [("olga", files.olga), ("igz", files.igz)] {
        let path = file(&dir, ext);
        if std::fs::read(&path).ok().as_deref() != Some(bytes) {
            std::fs::write(&path, bytes).ok()?;
        }
    }
    // The project keeps what was changed while trying the example out, until a
    // new Tapetum brings a changed example: then it starts afresh.
    let project = file(&dir, tapetum_project::EXTENSION);
    let stamp = dir.join(".example-version");
    let version = fingerprint(files);
    if !project.exists() || std::fs::read_to_string(&stamp).ok().as_deref() != Some(version.as_str()) {
        std::fs::write(&project, files.project).ok()?;
        std::fs::write(&stamp, &version).ok()?;
    }
    Some(file(&dir, "olga"))
}

/// Changes whenever any of the example's files does (FNV-1a over all three).
fn fingerprint(files: &Files) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in [files.olga, files.igz, files.project].concat() {
        h = (h ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Before the example in Tapetum's folder opens: if another program holds its
/// Art-Net port, project and mirror interfaces move to a free one, so it works
/// at once all the same.
pub fn ensure_free_port(show: &Path, bind: std::net::Ipv4Addr) {
    let project_path = tapetum_project::Project::path_for_show(show);
    let Ok(mut project) = tapetum_project::Project::load_or_default(&project_path) else { return };
    let port = bk_artnet::free_port_after(bind, project.artnet.listen_port, crate::app::PORT_GRACE);
    // The show is written afresh on every start, with mirrors on the default port.
    if port != bk_artnet::DEFAULT_PORT && set_mirror_port(show, port).is_err() {
        return;
    }
    if port != project.artnet.listen_port {
        project.artnet.listen_port = port;
        let _ = project.save(&project_path);
    }
}

/// Points the show's Tapetum interfaces at `port` (BlinderKitten has it closed).
fn set_mirror_port(show: &Path, port: u16) -> anyhow::Result<()> {
    let mut doc: serde_json::Value = serde_json::from_slice(&std::fs::read(show)?)?;
    bk_olga::mirror::add_mirrors(&mut doc, port)?;
    std::fs::write(show, serde_json::to_string_pretty(&doc)?)?;
    Ok(())
}

/// Is this show the example in Tapetum's own folder?
pub fn is_internal(show: &Path) -> bool {
    [&DE, &EN].into_iter().filter_map(internal_dir).any(|d| show.parent().is_some_and(|p| p == d))
}

/// Is this show a saved copy of the example?
pub fn is_saved_copy(show: &Path) -> bool {
    !is_internal(show) && show.file_stem().is_some_and(|s| s == NAME)
}

/// The files `save_to` writes into `dir` that are there already (links too,
/// even those pointing nowhere).
pub fn existing_in(dir: &Path) -> Vec<PathBuf> {
    ["olga", "igz", tapetum_project::EXTENSION].iter().map(|e| file(dir, e)).filter(|p| std::fs::symlink_metadata(p).is_ok()).collect()
}

/// Writes `bytes` as a plain file of its own: a symbolic link at `path` is
/// replaced, never written through (the chosen folder, say Downloads, may
/// hold a link named like the example pointing at some other file).
fn write_plain(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        std::fs::remove_file(path)?;
    }
    std::fs::write(path, bytes)
}

/// Writes the example into `dir` (the project as it is now, with what was
/// changed while trying it) and returns the show there.
pub fn save_to(dir: &Path, current_project: &Path) -> std::io::Result<PathBuf> {
    let files = current();
    write_plain(&file(dir, "olga"), files.olga)?;
    write_plain(&file(dir, "igz"), files.igz)?;
    let project = std::fs::read(current_project).unwrap_or_else(|_| files.project.to_vec());
    write_plain(&file(dir, tapetum_project::EXTENSION), &project)?;
    // The mirrors send to the port Tapetum listens on now.
    let port = serde_json::from_slice::<tapetum_project::Project>(&project).map_or(bk_artnet::DEFAULT_PORT, |p| p.artnet.listen_port);
    if port != bk_artnet::DEFAULT_PORT {
        set_mirror_port(&file(dir, "olga"), port).map_err(std::io::Error::other)?;
    }
    Ok(file(dir, "olga"))
}

/// Shows a file in Finder / Explorer / the file manager.
pub fn reveal(path: &Path) {
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-R").arg(path).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn()
    };
    if let Err(e) = result {
        eprintln!("{e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_loads() {
        for files in [&DE, &EN] {
            check(files);
        }
    }

    fn check(files: &Files) {
        let dir = std::env::temp_dir().join(format!("tapetum-example-{}-{}", files.lang, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (ext, bytes) in [("olga", files.olga), ("igz", files.igz), (tapetum_project::EXTENSION, files.project)] {
            std::fs::write(file(&dir, ext), bytes).unwrap();
        }
        let show_path = file(&dir, "olga");
        let show = bk_olga::load(&show_path).unwrap();
        assert_eq!(show.fixtures.len(), 15, "the test show's 13 and two front lights");
        assert!(show.fixtures.iter().all(|f| f.has_placement), "every fixture hangs somewhere");
        // Ready for Tapetum: every DMX interface has its mirror, no setup step needed.
        for (i, itf) in show.interfaces.iter().enumerate() {
            if !matches!(itf.kind, bk_olga::InterfaceKind::NonDmx(_)) && !bk_olga::mirror::is_mirror_name(&itf.name) {
                assert!(show.mirror_of(i).is_some(), "{} has no mirror", itf.name);
                if let bk_olga::InterfaceKind::ArtNet(out) = &itf.kind {
                    assert!(!out.enabled, "{} sends into the network", itf.name);
                }
            }
        }
        let project = tapetum_project::Project::load_or_default(&tapetum_project::Project::path_for_show(&show_path)).unwrap();
        assert_eq!(project.objects.len(), 1, "one IngeTrazo file, nothing else");
        let model = crate::stage::load(&dir.join(&project.objects[0].model), 1.0, tapetum_project::UpAxis::Z).unwrap();
        for scene in &project.scenes {
            for key in scene.objects.keys() {
                let part = key.split_once('/').unwrap().1;
                assert!(model.objects.iter().any(|o| o.key == part), "scene object {key} is in the .igz");
            }
        }
        assert_eq!(model.objects.iter().filter(|o| o.billboard).count(), 4, "four face-me figures");
        assert_eq!(show.conductor().map(|c| c.cues.len()), Some(8));
        assert_eq!(project.scenes.len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn taken_port_moves_example_and_saved_copy() {
        let base = std::env::temp_dir().join(format!("tapetum-example-port-{}", std::process::id()));
        let (here, there) = (base.join("internal"), base.join("saved"));
        std::fs::create_dir_all(&here).unwrap();
        std::fs::create_dir_all(&there).unwrap();
        let files = &DE;
        for (ext, bytes) in [("olga", files.olga), ("igz", files.igz)] {
            std::fs::write(file(&here, ext), bytes).unwrap();
        }
        // Another program holds the port the project wants.
        let other = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        let taken = other.local_addr().unwrap().port();
        let project_path = file(&here, tapetum_project::EXTENSION);
        let mut project: tapetum_project::Project = serde_json::from_slice(files.project).unwrap();
        project.artnet.listen_port = taken;
        project.save(&project_path).unwrap();

        let show = file(&here, "olga");
        ensure_free_port(&show, std::net::Ipv4Addr::UNSPECIFIED);
        let moved = tapetum_project::Project::load_or_default(&project_path).unwrap().artnet.listen_port;
        assert_ne!(moved, taken);
        let mirror_ports = |show: &Path| -> Vec<u16> {
            let s = bk_olga::load(show).unwrap();
            s.interfaces.iter().filter(|i| bk_olga::mirror::is_mirror_name(&i.name)).filter_map(|i| match &i.kind {
                bk_olga::InterfaceKind::ArtNet(a) => Some(a.port),
                _ => None,
            }).collect()
        };
        assert!(mirror_ports(&show).iter().all(|&p| p == moved) && !mirror_ports(&show).is_empty());

        let saved = save_to(&there, &project_path).unwrap();
        assert!(mirror_ports(&saved).iter().all(|&p| p == moved));

        // Next start: the show is the original again, the project keeps the moved port.
        std::fs::write(&show, files.olga).unwrap();
        ensure_free_port(&show, std::net::Ipv4Addr::UNSPECIFIED);
        assert!(mirror_ports(&show).iter().all(|&p| p == moved));
        drop(other);
        std::fs::remove_dir_all(&base).unwrap();
    }


    #[cfg(unix)]
    #[test]
    fn saving_replaces_links_instead_of_writing_through() {
        let base = std::env::temp_dir().join(format!("tapetum-example-link-{}", std::process::id()));
        let (folder, elsewhere) = (base.join("folder"), base.join("elsewhere.txt"));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(&elsewhere, b"keep me").unwrap();
        std::os::unix::fs::symlink(&elsewhere, file(&folder, "olga")).unwrap();
        std::os::unix::fs::symlink(base.join("nowhere"), file(&folder, "igz")).unwrap();
        assert_eq!(existing_in(&folder).len(), 2, "both links are asked about");

        let show = save_to(&folder, Path::new("/nonexistent")).unwrap();
        assert_eq!(std::fs::read(&elsewhere).unwrap(), b"keep me");
        assert!(!base.join("nowhere").exists());
        assert!(std::fs::symlink_metadata(&show).unwrap().is_file());
        assert!(bk_olga::load(&show).is_ok());
        std::fs::remove_dir_all(&base).unwrap();
    }

}
