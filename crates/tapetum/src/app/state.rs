use std::path::PathBuf;
use crate::i18n::tr;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use bk_olga::{Attribute, Resolution, Show};
use bk_olga::watch::ShowWatcher;
use tapetum_project::{Placement, Project};

use super::fixtures::FixtureSpec;
use crate::bk_link::{Edit, HostInfo, Progress};

#[derive(Resource)]
pub struct ShowState {
    pub show_path: PathBuf,
    pub show: Show,
    pub project: Project,
    pub project_path: PathBuf,
    /// Bumped whenever fixtures need to be rebuilt (reload, class/cell settings, quality).
    pub generation: u64,
    /// Resolved per-fixture settings of the current build, by fixture index.
    pub specs: Vec<Option<FixtureSpec>>,
    /// Fixture whose type the editor shows (the one clicked last).
    pub selected: Option<usize>,
    /// Fixtures that move together, by index; contains `selected`.
    pub selection: Vec<usize>,
    /// Group (index into `show.groups`) the selection came from.
    pub selection_group: Option<usize>,
    /// Stage object whose settings the editor shows (the last clicked), `"<file>/<object>"`.
    pub selected_object: Option<String>,
    /// All selected stage objects (contains `selected_object`).
    pub selected_objects: Vec<String>,
    /// Where a Shift click in the panel lists starts its range.
    pub anchor_fixture: Option<usize>,
    pub anchor_group: Option<usize>,
    pub anchor_object: Option<String>,
    /// Scene chosen in the panel while the main cuelist doesn't run (`None` = base scene).
    pub manual_scene: Option<u64>,
    /// Cell the LED painter assigns.
    pub led_brush: u32,
    pub status: Option<String>,
}

impl ShowState {
    pub fn select_object(&mut self, key: String) {
        self.clear_selection();
        self.selected_objects = vec![key.clone()];
        self.selected_object = Some(key);
    }

    /// Adds or removes one stage object (Shift/Cmd).
    pub fn toggle_object(&mut self, key: String) {
        self.selected = None;
        self.selection.clear();
        self.selection_group = None;
        if let Some(i) = self.selected_objects.iter().position(|k| *k == key) {
            self.selected_objects.remove(i);
            if self.selected_object.as_ref() == Some(&key) {
                self.selected_object = self.selected_objects.last().cloned();
            }
        } else {
            self.selected_objects.push(key.clone());
            self.selected_object = Some(key);
        }
    }

    /// Several stage objects at once (a Shift range); `primary` is shown in the editor.
    pub fn select_objects(&mut self, keys: Vec<String>, primary: String) {
        self.clear_selection();
        self.selected_objects = keys;
        self.selected_object = Some(primary);
    }

    /// Several fixtures at once (a Shift range).
    pub fn select_fixtures(&mut self, list: Vec<usize>, primary: usize) {
        self.clear_selection();
        self.selection = list;
        self.selected = Some(primary);
    }

    /// Several groups at once (a Shift range): all their fixtures.
    pub fn select_groups(&mut self, groups: &[usize], primary: usize) {
        self.clear_selection();
        let ids: Vec<i64> = groups.iter().flat_map(|&g| self.show.groups[g].fixtures.iter().copied()).collect();
        self.selection = self.show.fixtures.iter().enumerate().filter(|(_, f)| ids.contains(&f.id)).map(|(i, _)| i).collect();
        self.selected = self.selection.first().copied();
        self.selection_group = (groups.len() == 1).then_some(primary);
    }

    pub fn select_fixture(&mut self, i: usize) {
        self.selected_object = None;
        self.selected_objects.clear();
        self.selected = Some(i);
        self.selection = vec![i];
        self.selection_group = None;
    }

    /// Selects group `g`; `primary` (a member) stays the one the editor shows.
    pub fn select_group(&mut self, g: usize, primary: Option<usize>) {
        self.selected_object = None;
        self.selected_objects.clear();
        let ids = &self.show.groups[g].fixtures;
        self.selection = self.show.fixtures.iter().enumerate().filter(|(_, f)| ids.contains(&f.id)).map(|(i, _)| i).collect();
        self.selected = primary.filter(|p| self.selection.contains(p)).or(self.selection.first().copied());
        self.selection_group = Some(g);
    }

    /// Adds the members of group `g` to the selection (Shift/Cmd).
    pub fn add_group(&mut self, g: usize, primary: Option<usize>) {
        if self.selection.is_empty() {
            return self.select_group(g, primary);
        }
        let ids = &self.show.groups[g].fixtures;
        let new: Vec<usize> = self.show.fixtures.iter().enumerate().filter(|(i, f)| ids.contains(&f.id) && !self.selection.contains(i)).map(|(i, _)| i).collect();
        self.selection.extend(new);
        if let Some(p) = primary.filter(|p| self.selection.contains(p)) {
            self.selected = Some(p);
        }
        self.selection_group = None;
    }

    pub fn clear_selection(&mut self) {
        self.selected_object = None;
        self.selected_objects.clear();
        self.selected = None;
        self.selection.clear();
        self.selection_group = None;
    }

    /// Adds or removes one fixture (Cmd/Ctrl + click).
    pub fn toggle_fixture(&mut self, i: usize) {
        self.selected_object = None;
        self.selected_objects.clear();
        if let Some(pos) = self.selection.iter().position(|&s| s == i) {
            self.selection.remove(pos);
            if self.selected == Some(i) {
                self.selected = self.selection.last().copied();
            }
        } else {
            self.selection.push(i);
            self.selected = Some(i);
        }
        self.selection_group = None;
    }

    /// Groups (indices) that contain fixture `i`.
    pub fn groups_of(&self, i: usize) -> Vec<usize> {
        let id = self.show.fixtures[i].id;
        self.show.groups.iter().enumerate().filter(|(_, g)| g.fixtures.contains(&id)).map(|(gi, _)| gi).collect()
    }

    /// Directory of the project file; relative asset paths start here.
    pub fn project_dir(&self) -> PathBuf {
        self.project_path.parent().map(PathBuf::from).unwrap_or_default()
    }

    pub fn save_project(&mut self) {
        self.status = self.project.save(&self.project_path).err().map(|e| e.to_string());
    }
}

#[derive(Resource)]
pub struct Dmx {
    pub receiver: bk_artnet::Receiver,
    pub bind: Ipv4Addr,
}

impl Dmx {
    /// Listens on another port; keeps the old receiver if the port is taken.
    pub fn restart(&mut self, port: u16) -> std::io::Result<()> {
        self.receiver = bk_artnet::Receiver::start(self.bind, port)?;
        Ok(())
    }
}

/// BK's remote control status, polled in the background, plus its live groups.
#[derive(Resource)]
pub struct BkStatus {
    info: Arc<Mutex<Option<HostInfo>>>,
    /// Latest `/group` answer and a counter that changes with every answer.
    groups: Arc<Mutex<(u64, Option<serde_json::Value>)>>,
    /// Control address of the main conductor cuelist (set from the show).
    conductor_path: Arc<Mutex<Option<String>>>,
    conductor: Arc<Mutex<Option<Conductor>>>,
    port: Arc<AtomicU16>,
}

/// Live state of the main conductor cuelist.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conductor {
    pub on: bool,
    pub current_id: f64,
}

impl BkStatus {
    pub fn start(port: u16) -> Self {
        let info = Arc::new(Mutex::new(None));
        let groups = Arc::new(Mutex::new((0, None)));
        let conductor_path = Arc::new(Mutex::new(None));
        let conductor = Arc::new(Mutex::new(None));
        let port = Arc::new(AtomicU16::new(port));
        let (i, g, cp, c, p) = (Arc::downgrade(&info), Arc::downgrade(&groups), conductor_path.clone(), Arc::downgrade(&conductor), port.clone());
        std::thread::spawn(move || {
            let mut tick = 0u32;
            // Ends when the resource is dropped.
            while let (Some(info), Some(groups), Some(conductor)) = (i.upgrade(), g.upgrade(), c.upgrade()) {
                let port = p.load(Ordering::Relaxed);
                // Status and groups every 2 s; the cue four times a second, so scenes follow quickly.
                if tick.is_multiple_of(8) {
                    let result = crate::bk_link::host_info(port);
                    let live = result.is_some().then(|| crate::bk_link::remote_json(port, "/group")).flatten();
                    if let Ok(mut slot) = info.lock() {
                        *slot = result;
                    }
                    if let (Some(json), Ok(mut slot)) = (live, groups.lock()) {
                        *slot = (slot.0 + 1, Some(json));
                    }
                }
                let connected = info.lock().ok().is_some_and(|i| i.is_some());
                let path = cp.lock().ok().and_then(|p| p.clone());
                let live = path.filter(|_| connected).and_then(|path| {
                    let value = |sub: &str| crate::bk_link::remote_json(port, &format!("/cuelist/{path}/{sub}"));
                    let first = |v: &serde_json::Value| v.get("VALUE").and_then(|v| v.get(0)).cloned();
                    let on = value("isOn").as_ref().and_then(first)?.as_bool()?;
                    let infos = value("conductorInfos")?;
                    let current_id = infos.get("CONTENTS")?.get("currentID").and_then(first)?.as_f64()?;
                    Some(Conductor { on, current_id })
                });
                if let Ok(mut slot) = conductor.lock() {
                    *slot = live;
                }
                drop((info, groups, conductor));
                tick = tick.wrapping_add(1);
                std::thread::sleep(Duration::from_millis(250));
            }
        });
        BkStatus { info, groups, conductor_path, conductor, port }
    }

    /// Live state of the main cuelist, if BK answers.
    pub fn conductor(&self) -> Option<Conductor> {
        *self.conductor.lock().ok()?
    }

    pub fn set_conductor_path(&self, path: Option<String>) {
        if let Ok(mut p) = self.conductor_path.lock()
            && *p != path
        {
            *p = path;
        }
    }

    /// The live groups if there is an answer newer than `seen`.
    pub fn groups_since(&self, seen: u64) -> Option<(u64, serde_json::Value)> {
        let slot = self.groups.lock().ok()?;
        (slot.0 != seen).then(|| slot.1.clone().map(|v| (slot.0, v))).flatten()
    }

    pub fn info(&self) -> Option<HostInfo> {
        self.info.lock().ok()?.clone()
    }

    pub fn set_port(&self, port: u16) {
        self.port.store(port, Ordering::Relaxed);
    }
}

#[derive(Resource)]
pub struct Watch(pub Mutex<ShowWatcher>);

impl Watch {
    pub fn new(path: &std::path::Path) -> notify::Result<Self> {
        Ok(Watch(Mutex::new(bk_olga::watch::watch(path, Duration::from_millis(300))?)))
    }
}

/// Picks up shows saved by BlinderKitten.
pub fn poll_reload(watch: Option<Res<Watch>>, mut state: ResMut<ShowState>, mut undo: ResMut<super::undo::UndoHistory>, mut offers: ResMut<LibraryUpdates>) {
    let Some(watch) = watch else { return };
    let Ok(w) = watch.0.lock() else { return };
    while let Ok(result) = w.updates.try_recv() {
        match result {
            Ok(show) => {
                // Ranges BK now has saved itself no longer need our copy.
                let mut project = state.project.clone();
                for ty in &show.fixture_types {
                    if let Some(o) = project.fixture_types.get_mut(&ty.short_name) {
                        for (attr, field) in [(Attribute::Pan, &mut o.pan_range), (Attribute::Tilt, &mut o.tilt_range), (Attribute::Zoom, &mut o.zoom_range)] {
                            let bk = super::fixtures::bk_range(ty, &attr);
                            if let (Some(a), Some(b)) = (*field, bk)
                                && (a[0] - b[0]).abs() < 1e-3
                                && (a[1] - b[1]).abs() < 1e-3
                            {
                                *field = None;
                            }
                        }
                    }
                }
                // New fixture types take what the fixture library has for them.
                let taken = crate::fixture_library::dir().map(|lib| crate::fixture_library::apply(&show, &mut project, &state.project_dir(), &lib)).unwrap_or_default();
                // Placements BK now has saved itself, too.
                project.pending_placements.retain(|id, p| {
                    let Some(f) = show.fixtures.iter().find(|f| f.id == *id) else { return true };
                    let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
                    !(f.has_placement && close(p.position, f.position) && close(p.rotation, f.rotation))
                });
                if project != state.project {
                    undo.external(&state.project, &project);
                    state.project = project;
                    state.save_project();
                }
                // New fixture types may have newer library files, too.
                *offers = LibraryUpdates::find(&show, &state.project);
                state.show = show;
                state.generation += 1;
                state.status = Some(if taken.is_empty() {
                    tr("Show neu geladen", "Show reloaded").into()
                } else {
                    trf!("Show neu geladen. Aus der Gerätebibliothek: {}", "Show reloaded. From the fixture library: {}", taken.join(", "))
                });
                let n = state.show.fixtures.len();
                if state.selected.is_some_and(|i| i >= n) || state.selection.iter().any(|&i| i >= n) {
                    state.clear_selection();
                }
            }
            // Keep the last good show; BK may still be writing.
            Err(e) => state.status = Some(trf!("Neuladen fehlgeschlagen: {e}", "Reloading failed: {e}", e = e)),
        }
    }
}

/// A running or finished show edit through BlinderKitten's remote control.
#[derive(Resource, Default)]
pub struct BkLink {
    pub job: Option<(Edit, Mutex<mpsc::Receiver<Progress>>)>,
    /// Edit waiting for the user to allow changing the file without BK.
    pub needs_offline: Option<Edit>,
    /// Last message and whether it is an error.
    pub message: Option<(String, bool)>,
}

impl BkLink {
    pub fn start(&mut self, state: &ShowState, edit: Edit, offline: bool) {
        let rx = crate::bk_link::spawn(state.show_path.clone(), state.project.artnet.bk_remote_port, edit, offline);
        self.job = Some((edit, Mutex::new(rx)));
        self.needs_offline = None;
        self.message = None;
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Collects progress of the running job.
    pub fn poll(&mut self) {
        let Some((edit, job)) = &self.job else { return };
        let edit = *edit;
        let Ok(rx) = job.lock() else { return };
        let mut finished = false;
        while let Ok(p) = rx.try_recv() {
            match p {
                Progress::Step(s) => self.message = Some((s, false)),
                Progress::Done(s) => {
                    self.message = Some((s, false));
                    finished = true;
                }
                Progress::Failed(s) => {
                    self.message = Some((s, true));
                    finished = true;
                }
                Progress::NoAnswer => {
                    self.message = None;
                    self.needs_offline = Some(edit);
                    finished = true;
                }
            }
        }
        drop(rx);
        if finished {
            self.job = None;
        }
    }
}

/// Physical ranges edited in the visualizer, on their way to BlinderKitten.
#[derive(Resource, Default)]
pub struct RangeSync {
    /// (fixture type short name, attribute) -> range in degrees.
    pending: Vec<(String, Attribute, [f32; 2])>,
    last_change: Option<Instant>,
    waiting_for_bk: bool,
}

impl RangeSync {
    pub fn queue(&mut self, ty: &str, attr: Attribute, range: [f32; 2]) {
        self.pending.retain(|(t, a, _)| !(t == ty && *a == attr));
        self.pending.push((ty.to_string(), attr, range));
        self.last_change = Some(Instant::now());
    }
}

/// Sends queued ranges as OSC to BK's remote control, once dragging paused briefly.
pub fn sync_ranges(mut sync: ResMut<RangeSync>, mut state: ResMut<ShowState>, bk: Res<BkStatus>) {
    if sync.pending.is_empty() || sync.last_change.is_some_and(|t| t.elapsed() < Duration::from_millis(150)) {
        return;
    }
    if !bk.info().is_some_and(|i| i.has_open(&state.show_path)) {
        if !sync.waiting_for_bk {
            sync.waiting_for_bk = true;
            state.status = Some(tr("BlinderKitten nicht verbunden – Pan/Tilt/Zoom-Bereiche werden übertragen, sobald die Show dort offen ist.", "BlinderKitten not connected – pan/tilt/zoom ranges go there as soon as the show is open.").into());
        }
        return;
    }
    let port = state.project.artnet.bk_remote_port;
    let mut sent = Vec::new();
    for (ty_name, attr, range) in std::mem::take(&mut sync.pending) {
        let Some(ty) = state.show.fixture_types.iter().find(|t| t.short_name == ty_name) else { continue };
        for ch in ty.channels.iter().filter(|c| c.attribute == attr && c.resolution != Resolution::FineOnly) {
            let address = Show::channel_address(ty, ch, "physicalRange");
            let args = [crate::bk_link::OscArg::Float(range[0]), crate::bk_link::OscArg::Float(range[1])];
            if let Err(e) = crate::bk_link::send_osc_args(port, &address, &args) {
                state.status = Some(trf!("Senden an BlinderKitten fehlgeschlagen: {e}", "Sending to BlinderKitten failed: {e}", e = e));
                return;
            }
        }
        sent.push(format!("{} {:?}", ty.name, attr));
    }
    sync.waiting_for_bk = false;
    if !sent.is_empty() {
        state.status = Some(trf!("In BlinderKitten eingetragen: {} – dort speichern, damit es bleibt.", "Entered in BlinderKitten: {} – save there to keep it.", sent.join(", ")));
    }
}

/// A gobo picture being picked in a file dialog.
#[derive(Resource, Default)]
pub struct GoboImport {
    job: Option<(String, usize, Mutex<mpsc::Receiver<Option<PathBuf>>>)>,
}

impl GoboImport {
    pub fn open(&mut self, ty: String, slot: usize) {
        if self.job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        // rfd hands the dialog to the main thread itself where the OS needs that.
        std::thread::spawn(move || {
            let file = rfd::FileDialog::new().set_title(tr("Gobo-Bild wählen", "Choose a gobo picture")).add_filter(tr("Bilder", "Pictures"), &["png", "jpg", "jpeg"]).pick_file();
            let _ = tx.send(file);
        });
        self.job = Some((ty, slot, Mutex::new(rx)));
    }

    /// Picked picture, copied next to the project: (type, slot, relative path or error).
    /// Takes `&ShowState` so polling doesn't mark it changed every frame.
    pub fn poll(&mut self, state: &ShowState) -> Option<(String, usize, Result<String, String>)> {
        let (_, _, rx) = self.job.as_ref()?;
        let file = rx.lock().ok()?.try_recv().ok()?;
        let (ty, slot, _) = self.job.take()?;
        Some((ty, slot, crate::app::gobo::import(&file?, &state.project_dir())))
    }
}

/// Library files newer than the project's settings of their fixture type, to
/// offer when the project opens; each with whether it is ticked.
#[derive(Resource, Default)]
pub struct LibraryUpdates {
    pub offers: Vec<(crate::fixture_library::Update, bool)>,
}

impl LibraryUpdates {
    pub fn find(show: &Show, project: &Project) -> Self {
        let offers = crate::fixture_library::dir().map(|lib| crate::fixture_library::updates(show, project, &lib)).unwrap_or_default();
        LibraryUpdates { offers: offers.into_iter().map(|u| (u, true)).collect() }
    }
}

/// What a fixture library dialog came back with.
pub enum LibraryDone {
    /// Saved: (type short name, file, fingerprint of the file's settings, its time stamp).
    Exported(String, PathBuf, String, u64),
    /// A file chosen for a type.
    Picked(String, PathBuf),
    Failed(String),
}

/// Exporting to and loading from the fixture library, with their dialogs in the background.
#[derive(Resource, Default)]
pub struct LibraryJobs {
    job: Option<Mutex<mpsc::Receiver<Option<LibraryDone>>>>,
}

impl LibraryJobs {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn start(&mut self, work: impl FnOnce() -> Option<LibraryDone> + Send + 'static) {
        if self.job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
        self.job = Some(Mutex::new(rx));
    }

    /// Saves a type's settings to the library, asking before replacing a file there.
    pub fn export(&mut self, short_name: String, type_name: String, settings: tapetum_project::FixtureOverride, project_dir: PathBuf, modified: u64) {
        self.start(move || {
            let Some(dir) = crate::fixture_library::dir() else { return Some(LibraryDone::Failed(tr("Kein Ordner für die Gerätebibliothek", "No folder for the fixture library").into())) };
            let file = crate::fixture_library::file_for(&dir, &type_name);
            if file.exists() {
                let answer = rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Warning)
                    .set_title(tr("Gerät ersetzen?", "Replace fixture?"))
                    .set_description(trf!("In der Gerätebibliothek gibt es „{}“ schon. Durch diese Einstellungen ersetzen?", "The fixture library already has “{}”. Replace it with these settings?", type_name))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show();
                if answer != rfd::MessageDialogResult::Yes {
                    return None;
                }
            }
            Some(match crate::fixture_library::export(&file, &type_name, &settings, &project_dir, modified) {
                Ok(source) => LibraryDone::Exported(short_name, file, source, modified),
                Err(e) => LibraryDone::Failed(e),
            })
        });
    }

    /// Lets the user choose a library file for a type.
    pub fn pick(&mut self, short_name: String) {
        self.start(move || {
            let mut dialog = rfd::FileDialog::new().set_title(tr("Gerät aus der Bibliothek laden", "Load a fixture from the library")).add_filter(tr("Tapetum-Gerät", "Tapetum fixture"), &[crate::fixture_library::EXTENSION]);
            if let Some(dir) = crate::fixture_library::dir().filter(|d| d.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            dialog.pick_file().map(|f| LibraryDone::Picked(short_name, f))
        });
    }

    pub fn poll(&mut self) -> Option<LibraryDone> {
        let rx = self.job.as_ref()?;
        let done = rx.lock().ok()?.try_recv().ok()?;
        self.job = None;
        done
    }
}

/// Fixture placements moved in the visualizer, on their way to BlinderKitten.
#[derive(Resource, Default)]
pub struct PlacementSync {
    /// Fixture ID -> (control address short name, placement), latest only.
    pending: std::collections::BTreeMap<i64, (String, Placement)>,
    last_send: Option<Instant>,
    connected: bool,
}

impl PlacementSync {
    pub fn queue(&mut self, id: i64, short_name: &str, p: Placement) {
        self.pending.insert(id, (short_name.to_string(), p));
    }
}

/// Sends moved placements as OSC (`/fixture/<name>/position` and `/rotation`),
/// at most ~30 times per second while dragging. When BK connects, placements
/// moved while it was away are sent too.
pub fn sync_placements(mut sync: ResMut<PlacementSync>, mut state: ResMut<ShowState>, bk: Res<BkStatus>) {
    let connected = bk.info().is_some_and(|i| i.has_open(&state.show_path));
    if connected && !sync.connected {
        for f in &state.show.fixtures {
            if let Some(p) = state.project.pending_placements.get(&f.id)
                && (p.position != f.position || p.rotation != f.rotation)
            {
                sync.pending.insert(f.id, (f.short_name.clone(), *p));
            }
        }
    }
    sync.connected = connected;
    if !connected || sync.pending.is_empty() || sync.last_send.is_some_and(|t| t.elapsed() < Duration::from_millis(33)) {
        return;
    }
    let port = state.project.artnet.bk_remote_port;
    for (_, (short, p)) in std::mem::take(&mut sync.pending) {
        for (param, v) in [("position", p.position), ("rotation", p.rotation)] {
            let args = v.map(crate::bk_link::OscArg::Float);
            if let Err(e) = crate::bk_link::send_osc_args(port, &format!("/fixture/{short}/{param}"), &args) {
                state.status = Some(trf!("Senden an BlinderKitten fehlgeschlagen: {e}", "Sending to BlinderKitten failed: {e}", e = e));
                return;
            }
        }
    }
    sync.last_send = Some(Instant::now());
}

/// Groups straight from the running BK, so new ones show up before BK saves.
pub fn live_groups(bk: Res<BkStatus>, mut state: ResMut<ShowState>, mut seen: Local<u64>) {
    if !bk.info().is_some_and(|i| i.has_open(&state.show_path)) {
        return;
    }
    let Some((n, json)) = bk.groups_since(*seen) else { return };
    *seen = n;
    let groups = bk_olga::groups_from_remote(&json, &state.show.fixtures);
    if groups.is_empty() && !state.show.groups.is_empty() {
        // An empty or odd answer shouldn't wipe what the file has.
        return;
    }
    if groups != state.show.groups {
        if state.selection_group.is_some_and(|g| g >= groups.len()) {
            state.clear_selection();
        }
        state.show.groups = groups;
    }
}

/// A stage model being picked in a file dialog.
#[derive(Resource, Default)]
pub struct ObjectImport {
    job: Option<Mutex<mpsc::Receiver<Option<PathBuf>>>>,
}

impl ObjectImport {
    pub fn open(&mut self) {
        if self.job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let file = rfd::FileDialog::new()
                .set_title(tr("Bühnenobjekt importieren", "Import stage object"))
                .add_filter(tr("3D-Modelle (IngeTrazo, glTF, OBJ, STL)", "3D models (IngeTrazo, glTF, OBJ, STL)"), &crate::stage::EXTENSIONS)
                .pick_file();
            let _ = tx.send(file);
        });
        self.job = Some(Mutex::new(rx));
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// The chosen file once the dialog closed (`None` while open or cancelled).
    pub fn poll(&mut self) -> Option<PathBuf> {
        let file = self.job.as_ref()?.lock().ok()?.try_recv().ok()?;
        self.job = None;
        file
    }
}
