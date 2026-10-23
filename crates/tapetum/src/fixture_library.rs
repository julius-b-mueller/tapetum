//! The fixture library: the settings made for a fixture type, saved once and
//! taken over by every project with a BlinderKitten fixture type of the same
//! name. One `<type name>.tapetum-fixture` per type in a folder of the user's
//! choice; its own gobo pictures in `<type name> gobos/` next to it.
//!
//! The project keeps its own copy of the settings either way, so it works
//! without the library. Taken over are only types whose settings weren't
//! changed in the project; a library file updated since is followed as long
//! as the project still has exactly what it took over.

use std::path::{Component, Path, PathBuf};

use bk_olga::Show;
use serde::{Deserialize, Serialize};
use tapetum_project::{FixtureOverride, Gobo, LibraryLink, Project};

pub const EXTENSION: &str = "tapetum-fixture";
const FORMAT: u32 = 1;

#[derive(Serialize, Deserialize)]
struct FixtureFile {
    format: u32,
    /// BlinderKitten's name of the fixture type, as it was exported.
    fixture_type: String,
    settings: FixtureOverride,
}

/// `Documents/Tapetum/Fixtures` in the user's home.
pub fn default_dir() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(home).join("Documents").join("Tapetum").join("Fixtures"))
}

/// The folder set in the settings, read once.
static CHOSEN: std::sync::Mutex<Option<Option<PathBuf>>> = std::sync::Mutex::new(None);

/// The library folder: the one set in the settings, else the default.
pub fn dir() -> Option<PathBuf> {
    let mut chosen = CHOSEN.lock().ok()?;
    chosen.get_or_insert_with(|| crate::i18n::UserSettings::load().fixture_library).clone().or_else(default_dir)
}

/// Sets the library folder (`None`: the default) and remembers it.
pub fn set_dir(dir: Option<PathBuf>) {
    let mut s = crate::i18n::UserSettings::load();
    s.fixture_library = dir.clone();
    s.save();
    if let Ok(mut chosen) = CHOSEN.lock() {
        *chosen = Some(dir);
    }
}

/// The library file for a fixture type of this BlinderKitten name.
pub fn file_for(dir: &Path, type_name: &str) -> PathBuf {
    dir.join(format!("{}.{EXTENSION}", file_stem(type_name)))
}

/// A type name made safe as a file name (no separators, nothing hidden).
fn file_stem(type_name: &str) -> String {
    let s: String = type_name.chars().map(|c| if c.is_alphanumeric() || " -_+().,".contains(c) { c } else { '_' }).collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() { "fixture".into() } else { s }
}

/// Fingerprint of settings (FNV-1a over their JSON).
pub fn fingerprint(o: &FixtureOverride) -> String {
    let json = serde_json::to_vec(o).unwrap_or_default();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in json {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Every gobo picture the settings use.
fn pictures(o: &mut FixtureOverride) -> impl Iterator<Item = &mut String> {
    o.fixed_gobo.iter_mut().chain(o.gobo_wheel.iter_mut().flatten().map(|s| &mut s.gobo)).filter_map(|g| match g {
        Gobo::Image(p) => Some(p),
        _ => None,
    })
}

/// A relative path that stays inside the folder it is relative to.
fn stays_inside(rel: &str) -> bool {
    Path::new(rel).components().all(|c| matches!(c, Component::Normal(_)))
}

/// Saves a fixture type's settings (and their pictures) as `file`; returns the
/// fingerprint of the file's settings.
pub fn export(file: &Path, type_name: &str, settings: &FixtureOverride, project_dir: &Path) -> Result<String, String> {
    let dir = file.parent().ok_or("no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let pictures_dir = format!("{stem} gobos");
    let mut settings = settings.clone();
    for p in pictures(&mut settings) {
        let src = project_dir.join(&*p);
        let Some(name) = src.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        if src.is_file() {
            std::fs::create_dir_all(dir.join(&pictures_dir)).map_err(|e| e.to_string())?;
            std::fs::copy(&src, dir.join(&pictures_dir).join(&name)).map_err(|e| e.to_string())?;
            *p = format!("{pictures_dir}/{name}");
        }
    }
    let source = fingerprint(&settings);
    let json = serde_json::to_string_pretty(&FixtureFile { format: FORMAT, fixture_type: type_name.to_string(), settings }).map_err(|e| e.to_string())?;
    std::fs::write(file, json).map_err(|e| e.to_string())?;
    Ok(source)
}

/// The settings in a library file as they are there (pictures relative to it).
fn read_raw(file: &Path) -> Result<FixtureOverride, String> {
    let bytes = crate::stage::read_limited(file, 4 << 20).map_err(|e| e.to_string())?;
    let f: FixtureFile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if f.format > FORMAT {
        return Err(trf!("Gerätedatei aus einer neueren Tapetum-Version", "Fixture file from a newer Tapetum"));
    }
    Ok(f.settings)
}

/// The settings of a library file for the project in `project_dir`: its
/// pictures copied next to the project (only real pictures, only from the
/// library's own folder), paths rewritten. Returns them with the fingerprint
/// of the file's own settings.
pub fn read(file: &Path, project_dir: &Path) -> Result<(FixtureOverride, String), String> {
    let raw = read_raw(file)?;
    let source = fingerprint(&raw);
    let mut settings = raw;
    let dir = file.parent().unwrap_or(Path::new("."));
    for p in pictures(&mut settings) {
        // A file from someone else must not point anywhere outside its folder.
        if !stays_inside(p) {
            *p = String::new();
            continue;
        }
        match crate::app::gobo::import(&dir.join(&*p), project_dir) {
            Ok(rel) => *p = rel,
            Err(_) => *p = String::new(),
        }
    }
    // Pictures that couldn't come along leave their slot open.
    if matches!(&settings.fixed_gobo, Some(Gobo::Image(p)) if p.is_empty()) {
        settings.fixed_gobo = None;
    }
    for s in settings.gobo_wheel.iter_mut().flatten() {
        if matches!(&s.gobo, Gobo::Image(p) if p.is_empty()) {
            s.gobo = Gobo::Open;
        }
    }
    Ok((settings, source))
}

/// Takes a library file over for one fixture type of the project.
pub fn take_over(project: &mut Project, short_name: &str, file: &Path, project_dir: &Path) -> Result<(), String> {
    let (settings, source) = read(file, project_dir)?;
    let applied = fingerprint(&settings);
    if settings == FixtureOverride::default() {
        project.fixture_types.remove(short_name);
    } else {
        project.fixture_types.insert(short_name.to_string(), settings);
    }
    project.library_links.insert(short_name.to_string(), LibraryLink { file: file.to_string_lossy().into_owned(), applied, source });
    Ok(())
}

/// When a project opens: takes the library over for every fixture type of the
/// show whose settings weren't changed in the project. Returns the names of
/// the types it took over.
pub fn apply(show: &Show, project: &mut Project, project_dir: &Path, library: &Path) -> Vec<String> {
    let mut taken = Vec::new();
    for ty in &show.fixture_types {
        let short = ty.short_name.as_str();
        let link = project.library_links.get(short).cloned();
        let file = link.as_ref().map_or_else(|| file_for(library, &ty.name), |l| PathBuf::from(&l.file));
        if !file.is_file() {
            continue;
        }
        let current = project.fixture_types.get(short).cloned().unwrap_or_default();
        let wanted = match &link {
            // Never taken over: only if nothing was set here.
            None => current == FixtureOverride::default(),
            // Taken over before: follow an updated file while unchanged here.
            Some(l) => fingerprint(&current) == l.applied && read_raw(&file).is_ok_and(|raw| fingerprint(&raw) != l.source),
        };
        if wanted && take_over(project, short, &file, project_dir).is_ok() {
            taken.push(ty.name.clone());
        }
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tapetum-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn show() -> Show {
        bk_olga::load(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../bk-olga/tests/show.olga"))).unwrap()
    }

    #[test]
    fn takes_over_unchanged_types_keeps_changed_ones_and_follows_updates() {
        let (library, project_dir) = (temp("lib"), temp("project"));
        let show = show();
        let ty = &show.fixture_types[1];
        let mine = FixtureOverride { beam_angle: Some(12.0), lumens: Some(4000.0), ..Default::default() };
        export(&file_for(&library, &ty.name), &ty.name, &mine, &project_dir).unwrap();

        // A new project takes it over.
        let mut project = Project::default();
        assert_eq!(apply(&show, &mut project, &project_dir, &library), vec![ty.name.clone()]);
        assert_eq!(project.fixture_types[&ty.short_name], mine);

        // The library file is updated: the unchanged project follows.
        let newer = FixtureOverride { beam_angle: Some(20.0), ..mine.clone() };
        export(&file_for(&library, &ty.name), &ty.name, &newer, &project_dir).unwrap();
        apply(&show, &mut project, &project_dir, &library);
        assert_eq!(project.fixture_types[&ty.short_name].beam_angle, Some(20.0));

        // Changed in the project: that wins, nothing is taken over any more.
        project.fixture_types.get_mut(&ty.short_name).unwrap().lumens = Some(999.0);
        export(&file_for(&library, &ty.name), &ty.name, &mine, &project_dir).unwrap();
        assert!(apply(&show, &mut project, &project_dir, &library).is_empty());
        assert_eq!(project.fixture_types[&ty.short_name].lumens, Some(999.0));

        // A project with its own settings and no link is left alone, too.
        let mut own = Project::default();
        own.fixture_types.insert(ty.short_name.clone(), FixtureOverride { lumens: Some(1.0), ..Default::default() });
        assert!(apply(&show, &mut own, &project_dir, &library).is_empty());
    }

    #[test]
    fn pictures_travel_along_and_cannot_leave_the_library() {
        let (library, from, to) = (temp("pic-lib"), temp("pic-from"), temp("pic-to"));
        std::fs::create_dir_all(from.join("tapetum-gobos")).unwrap();
        image::GrayImage::from_pixel(8, 8, image::Luma([200])).save(from.join("tapetum-gobos/star.png")).unwrap();
        let o = FixtureOverride { fixed_gobo: Some(Gobo::Image("tapetum-gobos/star.png".into())), ..Default::default() };
        let file = file_for(&library, "Spot / Profil");
        assert!(!file.file_name().unwrap().to_string_lossy().contains('/'));
        export(&file, "Spot / Profil", &o, &from).unwrap();
        let (got, _) = read(&file, &to).unwrap();
        assert_eq!(got.fixed_gobo, Some(Gobo::Image("tapetum-gobos/star.png".into())));
        assert!(to.join("tapetum-gobos/star.png").is_file());

        // A planted file pointing outside its folder brings nothing along.
        let evil = FixtureFile { format: FORMAT, fixture_type: "x".into(), settings: FixtureOverride { fixed_gobo: Some(Gobo::Image("../../secret.png".into())), ..Default::default() } };
        std::fs::write(library.join("evil.tapetum-fixture"), serde_json::to_vec(&evil).unwrap()).unwrap();
        let (got, _) = read(&library.join("evil.tapetum-fixture"), &to).unwrap();
        assert_eq!(got.fixed_gobo, None);
    }
}
