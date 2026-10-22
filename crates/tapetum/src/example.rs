//! The example that ships with Tapetum: a small theatre as an IngeTrazo file, a
//! BlinderKitten show with fixtures and cues, and its project. It opens on the
//! first start; saved into a folder of the user's choice, its files open in
//! BlinderKitten and IngeTrazo like any others. Built by `scripts/gen-example.py`.

use std::path::{Path, PathBuf};

pub const NAME: &str = "Tapetum-Demo";
const OLGA: &[u8] = include_bytes!("../assets/example/Tapetum-Demo.olga");
const IGZ: &[u8] = include_bytes!("../assets/example/Tapetum-Demo.igz");
const PROJECT: &[u8] = include_bytes!("../assets/example/Tapetum-Demo.tapetum");

fn file(dir: &Path, ext: &str) -> PathBuf {
    dir.join(format!("{NAME}.{ext}"))
}

/// Where the example lives until it is saved somewhere.
fn internal_dir() -> Option<PathBuf> {
    Some(crate::i18n::config_dir()?.join("example"))
}

/// The example's show in Tapetum's own folder, written there first; the
/// project is kept, so changes made while trying it out stay.
pub fn open_internal() -> Option<PathBuf> {
    let dir = internal_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    for (ext, bytes) in [("olga", OLGA), ("igz", IGZ)] {
        let path = file(&dir, ext);
        if std::fs::read(&path).ok().as_deref() != Some(bytes) {
            std::fs::write(&path, bytes).ok()?;
        }
    }
    let project = file(&dir, tapetum_project::EXTENSION);
    if !project.exists() {
        std::fs::write(&project, PROJECT).ok()?;
    }
    Some(file(&dir, "olga"))
}

/// Is this show the example in Tapetum's own folder?
pub fn is_internal(show: &Path) -> bool {
    internal_dir().is_some_and(|d| show.parent().is_some_and(|p| p == d))
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
    std::fs::write(file(dir, "olga"), OLGA)?;
    std::fs::write(file(dir, "igz"), IGZ)?;
    let project = std::fs::read(current_project).unwrap_or_else(|_| PROJECT.to_vec());
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
        let dir = std::env::temp_dir().join(format!("tapetum-example-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let show_path = save_to(&dir, Path::new("/nonexistent")).unwrap();
        let show = bk_olga::load(&show_path).unwrap();
        assert_eq!(show.fixtures.len(), 13);
        assert!(show.fixtures.iter().all(|f| f.has_placement), "every fixture hangs somewhere");
        let project = tapetum_project::Project::load_or_default(&tapetum_project::Project::path_for_show(&show_path)).unwrap();
        assert_eq!(project.objects.len(), 1, "one IngeTrazo file, nothing else");
        let model = crate::stage::load(&dir.join(&project.objects[0].model), 1.0, tapetum_project::UpAxis::Z).unwrap();
        for scene in &project.scenes {
            for key in scene.objects.keys() {
                let part = key.split_once('/').unwrap().1;
                assert!(model.objects.iter().any(|o| o.key == part), "scene object {key} is in the .igz");
            }
        }
        assert!(model.objects.iter().any(|o| o.billboard), "the face-me figure");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
