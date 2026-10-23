//! The fixture library: the settings made for a fixture type, saved once and
//! taken over by every project with a BlinderKitten fixture type of the same
//! name. One `<type name>.tapetum-fixture` per type in a folder of the user's
//! choice; its own gobo pictures in `<type name> gobos/` next to it, and the
//! type itself for BlinderKitten as `<type name>.mochi` (File → Import there).
//!
//! The project keeps its own copy of the settings either way, so it works
//! without the library. Types not set up in the project take the library over
//! by themselves; for the others, library files changed later than the
//! project's settings are offered (see [`updates`]), each until taken over or
//! declined. Both sides carry the time their settings last changed.

use std::path::{Component, Path, PathBuf};

use bk_olga::Show;
use serde::{Deserialize, Serialize};
use tapetum_project::{FixtureOverride, Gobo, LibraryLink, Project, now};

pub const EXTENSION: &str = "tapetum-fixture";
const FORMAT: u32 = 1;

#[derive(Serialize, Deserialize)]
struct FixtureFile {
    format: u32,
    /// BlinderKitten's name of the fixture type, as it was exported.
    fixture_type: String,
    /// When these settings last changed (Unix seconds; 0 in files without it).
    #[serde(default)]
    modified: u64,
    settings: FixtureOverride,
}

/// `Documents/Tapetum/Fixtures` in the user's home.
pub fn default_dir() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(home).join("Documents").join("Tapetum").join("Fixtures"))
}

/// The folder set in the settings, read once.
static CHOSEN: std::sync::Mutex<Option<Option<PathBuf>>> = std::sync::Mutex::new(None);

/// The library folder: `TAPETUM_FIXTURE_LIBRARY` (for tests), the one set in
/// the settings, else the default.
pub fn dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("TAPETUM_FIXTURE_LIBRARY") {
        return Some(PathBuf::from(d));
    }
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

/// Saves a fixture type's settings (and their pictures) as `file`, last changed
/// at `modified`; returns the fingerprint of the file's settings.
pub fn export(file: &Path, type_name: &str, settings: &FixtureOverride, project_dir: &Path, modified: u64) -> Result<String, String> {
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
    let json = serde_json::to_string_pretty(&FixtureFile { format: FORMAT, fixture_type: type_name.to_string(), modified, settings }).map_err(|e| e.to_string())?;
    std::fs::write(file, json).map_err(|e| e.to_string())?;
    Ok(source)
}

/// The fixture type of a BlinderKitten show as a `.mochi` (what BK's File →
/// Export Selection writes for it): the type and the channel families its
/// channels use. BK merges families by name when importing.
pub fn mochi(show_path: &Path, type_name: &str) -> Result<String, String> {
    let bytes = crate::stage::read_limited(show_path, 256 << 20).map_err(|e| e.to_string())?;
    let show: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let items = |key: &str| show.get(key).and_then(|m| m.get("items")).and_then(serde_json::Value::as_array).cloned().unwrap_or_default();
    let ty = items("fixtureType")
        .into_iter()
        .find(|t| t.get("niceName").and_then(serde_json::Value::as_str) == Some(type_name))
        .ok_or_else(|| trf!("Gerätetyp „{}“ nicht in der Show", "Fixture type “{}” not in the show", type_name))?;
    // The families its channel types belong to: "/<family>/channelTypes/<type>".
    let mut used = std::collections::BTreeSet::new();
    let mut walk = vec![&ty];
    while let Some(v) = walk.pop() {
        match v {
            serde_json::Value::Object(m) => {
                if m.get("controlAddress").and_then(serde_json::Value::as_str) == Some("/channelType")
                    && let Some(family) = m.get("value").and_then(serde_json::Value::as_str).and_then(|a| a.strip_prefix('/')).and_then(|a| a.split('/').next())
                {
                    used.insert(family.to_string());
                }
                walk.extend(m.values());
            }
            serde_json::Value::Array(a) => walk.extend(a.iter()),
            _ => {}
        }
    }
    let families: Vec<serde_json::Value> = items("channelFamilies")
        .into_iter()
        .filter(|f| f.get("niceName").and_then(serde_json::Value::as_str).is_some_and(|n| used.contains(&bk_olga::short_name::to_short_name(n, false))))
        .collect();
    serde_json::to_string(&serde_json::json!({ "fixtureType": [ty], "channelFamilies": families })).map_err(|e| e.to_string())
}

/// The `.mochi` that goes with a library file.
pub fn mochi_for(file: &Path) -> PathBuf {
    file.with_extension("mochi")
}

/// The settings in a library file as they are there (pictures relative to
/// it), and when they last changed.
fn read_raw(file: &Path) -> Result<(FixtureOverride, u64), String> {
    let bytes = crate::stage::read_limited(file, 4 << 20).map_err(|e| e.to_string())?;
    let f: FixtureFile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if f.format > FORMAT {
        return Err(trf!("Gerätedatei aus einer neueren Tapetum-Version", "Fixture file from a newer Tapetum"));
    }
    Ok((f.settings, f.modified))
}

/// The settings of a library file for the project in `project_dir`: its
/// pictures copied next to the project (only real pictures, only from the
/// library's own folder), paths rewritten. Returns them with the fingerprint
/// of the file's own settings and their time stamp.
pub fn read(file: &Path, project_dir: &Path) -> Result<(FixtureOverride, String, u64), String> {
    let (raw, modified) = read_raw(file)?;
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
    Ok((settings, source, modified))
}

/// Takes a library file over for one fixture type of the project.
pub fn take_over(project: &mut Project, short_name: &str, file: &Path, project_dir: &Path) -> Result<(), String> {
    let (settings, source, modified) = read(file, project_dir)?;
    let applied = fingerprint(&settings);
    // The project's settings are now as old as the library's.
    let modified = if modified > 0 { modified } else { now() };
    project.fixture_types_changed.insert(short_name.to_string(), modified);
    if settings == FixtureOverride::default() {
        project.fixture_types.remove(short_name);
    } else {
        project.fixture_types.insert(short_name.to_string(), settings);
    }
    project.library_links.insert(short_name.to_string(), LibraryLink { file: file.to_string_lossy().into_owned(), applied, source, seen: modified });
    Ok(())
}

/// A library file newer than a fixture type's settings in the project.
#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub short_name: String,
    pub type_name: String,
    pub file: PathBuf,
    /// When the library's settings changed.
    pub library: u64,
    /// When the project's settings changed (0: unknown, older than time stamps).
    pub project: u64,
}

/// The library files changed later than the project's settings of their
/// fixture type (and not declined in that version), with other settings.
pub fn updates(show: &Show, project: &Project, library: &Path) -> Vec<Update> {
    let mut list = Vec::new();
    for ty in &show.fixture_types {
        let short = ty.short_name.as_str();
        let link = project.library_links.get(short);
        let file = link.map_or_else(|| file_for(library, &ty.name), |l| PathBuf::from(&l.file));
        let Ok((raw, modified)) = read_raw(&file) else { continue };
        let changed = project.fixture_types_changed.get(short).copied().unwrap_or(0);
        let seen = link.map_or(0, |l| l.seen);
        let current = project.fixture_types.get(short).cloned().unwrap_or_default();
        let different = link.map_or_else(|| fingerprint(&current) != fingerprint(&raw), |l| fingerprint(&raw) != l.source || fingerprint(&current) != l.applied);
        if modified > changed.max(seen) && different {
            list.push(Update { short_name: short.to_string(), type_name: ty.name.clone(), file, library: modified, project: changed });
        }
    }
    list
}

/// Not now: this version of the library file isn't offered again.
pub fn decline(project: &mut Project, update: &Update) {
    let link = project.library_links.entry(update.short_name.clone()).or_insert_with(|| LibraryLink { file: update.file.to_string_lossy().into_owned(), ..Default::default() });
    link.seen = update.library;
}

/// When a project opens: takes the library over for every fixture type of the
/// show that isn't set up in the project (newer files for the others are
/// offered by [`updates`]). Returns the names of the types it took over.
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
        let wanted = link.is_none() && current == FixtureOverride::default();
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
    fn new_types_take_the_library_newer_files_are_offered() {
        let (library, project_dir) = (temp("lib"), temp("project"));
        let show = show();
        let ty = &show.fixture_types[1];
        let file = file_for(&library, &ty.name);
        let mine = FixtureOverride { beam_angle: Some(12.0), lumens: Some(4000.0), ..Default::default() };
        export(&file, &ty.name, &mine, &project_dir, 1000).unwrap();

        // A new project takes it over by itself, as old as the file.
        let mut project = Project::default();
        assert_eq!(apply(&show, &mut project, &project_dir, &library), vec![ty.name.clone()]);
        assert_eq!(project.fixture_types[&ty.short_name], mine);
        assert_eq!(project.fixture_types_changed[&ty.short_name], 1000);
        assert!(updates(&show, &project, &library).is_empty());

        // A newer file is offered, not taken over by itself.
        let newer = FixtureOverride { beam_angle: Some(20.0), ..mine.clone() };
        export(&file, &ty.name, &newer, &project_dir, 2000).unwrap();
        assert!(apply(&show, &mut project, &project_dir, &library).is_empty());
        let offered = updates(&show, &project, &library);
        assert_eq!(offered.len(), 1);
        assert_eq!((offered[0].library, offered[0].project), (2000, 1000));

        // Declined: not offered again in that version, but in the next one.
        decline(&mut project, &offered[0]);
        assert!(updates(&show, &project, &library).is_empty());
        let third = FixtureOverride { beam_angle: Some(30.0), ..mine.clone() };
        export(&file, &ty.name, &third, &project_dir, 3000).unwrap();
        let offered = updates(&show, &project, &library);
        assert_eq!(offered.len(), 1);

        // Taken over: the project has it, nothing is offered.
        take_over(&mut project, &ty.short_name, &offered[0].file, &project_dir).unwrap();
        assert_eq!(project.fixture_types[&ty.short_name], third);
        assert!(updates(&show, &project, &library).is_empty());

        // Saved again with the very settings the project has: nothing to offer.
        export(&file, &ty.name, &third, &project_dir, 3200).unwrap();
        assert!(updates(&show, &project, &library).is_empty());

        // Changed in the project later than the file: nothing to offer.
        project.fixture_types.get_mut(&ty.short_name).unwrap().lumens = Some(999.0);
        project.fixture_types_changed.insert(ty.short_name.clone(), 4000);
        export(&file, &ty.name, &newer, &project_dir, 3500).unwrap();
        assert!(updates(&show, &project, &library).is_empty());

        // A project with its own settings and no link is left alone, until the file is newer.
        let mut own = Project::default();
        own.fixture_types.insert(ty.short_name.clone(), FixtureOverride { lumens: Some(1.0), ..Default::default() });
        own.fixture_types_changed.insert(ty.short_name.clone(), 5000);
        assert!(apply(&show, &mut own, &project_dir, &library).is_empty());
        assert!(updates(&show, &own, &library).is_empty());
    }

    #[test]
    fn pictures_travel_along_and_cannot_leave_the_library() {
        let (library, from, to) = (temp("pic-lib"), temp("pic-from"), temp("pic-to"));
        std::fs::create_dir_all(from.join("tapetum-gobos")).unwrap();
        image::GrayImage::from_pixel(8, 8, image::Luma([200])).save(from.join("tapetum-gobos/star.png")).unwrap();
        let o = FixtureOverride { fixed_gobo: Some(Gobo::Image("tapetum-gobos/star.png".into())), ..Default::default() };
        let file = file_for(&library, "Spot / Profil");
        assert!(!file.file_name().unwrap().to_string_lossy().contains('/'));
        export(&file, "Spot / Profil", &o, &from, 1).unwrap();
        let (got, _, _) = read(&file, &to).unwrap();
        assert_eq!(got.fixed_gobo, Some(Gobo::Image("tapetum-gobos/star.png".into())));
        assert!(to.join("tapetum-gobos/star.png").is_file());

        // A planted file pointing outside its folder brings nothing along.
        let evil = FixtureFile { format: FORMAT, fixture_type: "x".into(), modified: 1, settings: FixtureOverride { fixed_gobo: Some(Gobo::Image("../../secret.png".into())), ..Default::default() } };
        std::fs::write(library.join("evil.tapetum-fixture"), serde_json::to_vec(&evil).unwrap()).unwrap();
        let (got, _, _) = read(&library.join("evil.tapetum-fixture"), &to).unwrap();
        assert_eq!(got.fixed_gobo, None);
    }

    #[test]
    fn mochi_has_the_type_and_its_channel_families() {
        let show_path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../bk-olga/tests/show.olga"));
        let show = show();
        let ty = show.fixture_types.iter().find(|t| t.name.contains("TMH")).unwrap();
        let m: serde_json::Value = serde_json::from_str(&mochi(show_path, &ty.name).unwrap()).unwrap();
        let types = m["fixtureType"].as_array().unwrap();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0]["niceName"], ty.name.as_str());
        let families: Vec<&str> = m["channelFamilies"].as_array().unwrap().iter().map(|f| f["niceName"].as_str().unwrap()).collect();
        for want in ["Dimmer", "Color", "Position", "Focus"] {
            assert!(families.contains(&want), "{families:?}");
        }
        assert!(!families.contains(&"Gobo"), "only the families it uses: {families:?}");
        assert_eq!(mochi_for(Path::new("/lib/X.tapetum-fixture")), Path::new("/lib/X.mochi"));
        assert!(mochi(show_path, "no such type").is_err());
    }

}
