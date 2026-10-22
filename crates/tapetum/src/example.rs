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
    let project = file(&dir, tapetum_project::EXTENSION);
    if !project.exists() {
        std::fs::write(&project, files.project).ok()?;
    }
    Some(file(&dir, "olga"))
}

/// Is this show the example in Tapetum's own folder?
pub fn is_internal(show: &Path) -> bool {
    [&DE, &EN].into_iter().filter_map(internal_dir).any(|d| show.parent().is_some_and(|p| p == d))
}

/// Is this show a saved copy of the example?
pub fn is_saved_copy(show: &Path) -> bool {
    !is_internal(show) && show.file_stem().is_some_and(|s| s == NAME)
}

/// The files `save_to` writes into `dir` that are there already.
pub fn existing_in(dir: &Path) -> Vec<PathBuf> {
    ["olga", "igz", tapetum_project::EXTENSION].iter().map(|e| file(dir, e)).filter(|p| p.exists()).collect()
}

/// Writes the example into `dir` (the project as it is now, with what was
/// changed while trying it) and returns the show there.
pub fn save_to(dir: &Path, current_project: &Path) -> std::io::Result<PathBuf> {
    let files = current();
    std::fs::write(file(dir, "olga"), files.olga)?;
    std::fs::write(file(dir, "igz"), files.igz)?;
    let project = std::fs::read(current_project).unwrap_or_else(|_| files.project.to_vec());
    std::fs::write(file(dir, tapetum_project::EXTENSION), project)?;
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
        assert_eq!(show.fixtures.len(), 13);
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
}
