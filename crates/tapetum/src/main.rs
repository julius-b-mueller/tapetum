// Bevy systems take their dependencies as parameters; long signatures are normal there.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]
// No console window next to the app on Windows (the subcommands are for developers).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod app;
mod bk_link;
mod example;
mod export;
mod fixture_library;
mod routing;
mod stage;

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bk_olga::{Attribute, Show, light};
use clap::{Parser, Subcommand};
use crate::i18n::tr;
use tapetum_project::Project;

#[derive(Parser)]
#[command(name = "tapetum", version, about = "Tapetum – 3D lighting visualizer for BlinderKitten")]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    view: ViewArgs,
}

/// Open the 3D view (default command).
#[derive(clap::Args)]
struct ViewArgs {
    /// BlinderKitten show file (.olga)
    show: Option<PathBuf>,
    /// Art-Net port (default: from the project file, normally 6454)
    #[arg(long)]
    port: Option<u16>,
    #[arg(long, default_value_t = Ipv4Addr::UNSPECIFIED)]
    bind: Ipv4Addr,
    /// Save a screenshot after N frames and quit, e.g. --screenshot out.png:120
    #[arg(long, value_parser = parse_screenshot)]
    screenshot: Option<(PathBuf, u32)>,
    /// Select the fixture with this ID at start (for screenshots).
    #[arg(long, hide = true)]
    select: Option<i64>,
    /// Select the stage object "<file id>/<object key>" at start (for screenshots).
    #[arg(long, hide = true)]
    select_object: Option<String>,
    /// Measure: no vsync, print the mean frame time of frames 300..600, then quit.
    #[arg(long, hide = true)]
    bench: bool,
    /// Show this scene at start (for screenshots).
    #[arg(long, hide = true)]
    scene: Option<u64>,
    /// Camera for screenshots: yaw,pitch,distance,target height[,target x,target y]
    /// (degrees, meters; x and y in BlinderKitten's coordinates, default the middle).
    #[arg(long, hide = true, value_parser = parse_orbit)]
    orbit: Option<[f32; 6]>,
    /// Language for this run (for screenshots): "de" or "en".
    #[arg(long, hide = true, value_parser = ["de", "en"])]
    lang: Option<String>,
    /// Open the about window at start (for screenshots).
    #[arg(long, hide = true)]
    about: bool,
    /// Start with the sidebar hidden (for screenshots).
    #[arg(long, hide = true)]
    no_sidebar: bool,
    /// Only the 3D view, without sidebar and overlays (for screenshots).
    #[arg(long, hide = true)]
    no_ui: bool,
    /// Open the settings at start (for screenshots).
    #[arg(long, hide = true)]
    settings: bool,
    /// Keep the faders and the scene bar out (for screenshots).
    #[arg(long, hide = true)]
    overlays_open: bool,
    /// Frame everything at start (for screenshots) from a standard view.
    #[arg(long, hide = true, value_parser = ["audience", "top", "front", "left", "right"])]
    fit: Option<String>,
    /// Export the fixture plan (.pdf or .svg) from the start view and quit (for testing).
    #[arg(long, hide = true)]
    export_plan: Option<PathBuf>,
    /// Export the set-change drawings (.pdf) from the start view and quit (for testing).
    #[arg(long, hide = true)]
    export_changes: Option<PathBuf>,
    /// Orthographic projection at start (for screenshots).
    #[arg(long, hide = true)]
    ortho: bool,
}

fn standard_view(name: &str) -> tapetum_project::StandardView {
    use tapetum_project::StandardView::*;
    match name {
        "top" => Top,
        "front" => Front,
        "left" => Left,
        "right" => Right,
        _ => Audience,
    }
}

fn parse_orbit(s: &str) -> Result<[f32; 6], String> {
    let mut v: Vec<f32> = s.split(',').map(|x| x.trim().parse::<f32>().map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
    if v.len() == 4 {
        v.extend([0.0, 0.0]);
    }
    v.try_into().map_err(|_| "yaw,pitch,distance,height[,x,y]".to_string())
}

fn parse_screenshot(s: &str) -> Result<(PathBuf, u32), String> {
    match s.rsplit_once(':') {
        Some((p, n)) => Ok((p.into(), n.parse().map_err(|e| format!("{e}"))?)),
        None => Ok((s.into(), 120)),
    }
}

#[derive(Subcommand)]
enum Command {
    /// Print fixtures, patch and detected fixture classes of a show.
    Dump { show: PathBuf },
    /// Print incoming Art-Net, decoded per fixture if a show is given.
    Monitor {
        #[arg(long)]
        show: Option<PathBuf>,
        #[arg(long, default_value_t = bk_artnet::DEFAULT_PORT)]
        port: u16,
        #[arg(long, default_value_t = Ipv4Addr::UNSPECIFIED)]
        bind: Ipv4Addr,
    },
    /// Send moving test patterns for every fixture of a show (no BlinderKitten needed).
    TestSend {
        show: PathBuf,
        #[arg(long, default_value = "127.0.0.1:6454")]
        target: SocketAddr,
        #[arg(long, default_value_t = 40.0)]
        fps: f32,
    },
    /// Add "Tapetum" Art-Net interfaces mirroring the show's DMX interfaces (via pass-through).
    /// Talks to BlinderKitten's OSC Remote Control so a running BK saves and reloads.
    Mirror {
        show: PathBuf,
        /// Remove the Tapetum interfaces instead.
        #[arg(long)]
        remove: bool,
        /// Edit the file without BlinderKitten (only while BK is closed).
        #[arg(long)]
        offline: bool,
    },
    /// Lay out Tapetum's 2D plans in BlinderKitten from the fixture positions
    /// (save → edit → reload, like `mirror`).
    Plans {
        show: PathBuf,
        /// Remove Tapetum's 2D plans instead.
        #[arg(long)]
        remove: bool,
        /// Edit the file without BlinderKitten (only while BK is closed).
        #[arg(long)]
        offline: bool,
    },
    /// List HID devices and print what a SpaceMouse reports (for diagnosis).
    #[command(hide = true)]
    Hid {
        /// Seconds to listen to the SpaceMouse.
        #[arg(long, default_value_t = 10)]
        seconds: u64,
    },
}

fn main() -> Result<()> {
    i18n::UserSettings::load().apply();
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Dump { show }) => dump(&show),
        Some(Command::Monitor { show, port, bind }) => monitor(show, bind, port),
        Some(Command::TestSend { show, target, fps }) => test_send(&show, target, fps),
        Some(Command::Mirror { show, remove, offline }) => {
            let (_, project) = load(&show)?;
            let edit = if remove { bk_link::Edit::RemoveMirrors } else { bk_link::Edit::AddMirrors { listen_port: project.artnet.listen_port } };
            edit_show(&show, edit, offline)
        }
        Some(Command::Plans { show, remove, offline }) => {
            let (s, project) = load(&show)?;
            let edit = if remove {
                bk_link::Edit::RemovePlans
            } else {
                bk_link::Edit::Plans { fixtures: app::layout::placements(&s, &project).iter().zip(&s.fixtures).map(|(p, f)| (f.id, p.position)).collect() }
            };
            edit_show(&show, edit, offline)
        }
        Some(Command::Hid { seconds }) => app::spacemouse_diagnose(seconds),
        None => {
            let v = cli.view;
            if let Some(l) = &v.lang {
                i18n::set(if l == "en" { i18n::Language::English } else { i18n::Language::German });
            }
            if v.no_sidebar {
                app::hide_sidebar_at_start();
            }
            if v.no_ui {
                app::hide_ui();
            }
            if v.screenshot.is_some() {
                app::onboarding::no_auto_open();
            }
            if v.settings {
                app::open_settings_at_start();
            }
            if v.overlays_open {
                app::keep_overlays_open();
            }
            if v.about {
                app::open_about_at_start();
            }
            // Started without a show (double click, dock): the last one, else the example.
            let mut settings = i18n::UserSettings::load();
            let show = v.show.or_else(|| settings.last_show.clone().filter(|p| p.is_file())).or_else(example::open_internal).or_else(pick_show);
            let Some(show) = show else { return Ok(()) };
            if example::is_internal(&show) {
                example::ensure_free_port(&show, v.bind);
            }
            // Remembered as an absolute path, but not for test runs (screenshots, benchmarks).
            let remembered = std::path::absolute(&show).unwrap_or_else(|_| show.clone());
            if v.screenshot.is_none() && !v.bench && settings.last_show.as_ref() != Some(&remembered) {
                settings.last_show = Some(remembered);
                settings.save();
            }
            app::run(app::Options { show, bind: v.bind, port: v.port, screenshot: v.screenshot, export: v
                    .export_plan
                    .map(|p| (if p.extension().is_some_and(|e| e == "svg") { app::exports::ExportKind::PlanSvg } else { app::exports::ExportKind::PlanPdf }, p))
                    .or(v.export_changes.map(|p| (app::exports::ExportKind::SetChangesPdf, p))),
                select: v.select, select_object: v.select_object, scene: v.scene, orbit: v.orbit, fit: v.fit.as_deref().map(standard_view), ortho: v.ortho, bench: v.bench })
        }
    }
}

/// Asks for a BlinderKitten show file.
pub fn pick_show() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title(tr("BlinderKitten-Show öffnen", "Open a BlinderKitten show"))
        .add_filter(tr("BlinderKitten-Show", "BlinderKitten show"), &["olga"])
        .pick_file()
}

fn edit_show(path: &PathBuf, edit: bk_link::Edit, offline: bool) -> Result<()> {
    let (_, project) = load(path)?;
    let report = |p| {
        if let bk_link::Progress::Step(s) = p {
            println!("{s}");
        }
    };
    match bk_link::run(path, project.artnet.bk_remote_port, edit, offline, &report)? {
        Some(done) => println!("{done}"),
        None => anyhow::bail!(
            "{}",
            trf!(
                "BlinderKitten hat nicht gespeichert. Ist die Show geöffnet und die OSC Remote Control \
                 (Einstellungen, Port {}) aktiv? Ist BK geschlossen, geht es mit --offline.",
                "BlinderKitten didn't save. Is the show open and OSC Remote Control \
                 (preferences, port {}) on? With BK closed, use --offline.",
                project.artnet.bk_remote_port
            )
        ),
    }
    Ok(())
}

fn load(path: &PathBuf) -> Result<(Show, Project)> {
    let show = bk_olga::load(path)?;
    let project = Project::load_or_default(&Project::path_for_show(path))?;
    Ok((show, project))
}

fn dump(path: &PathBuf) -> Result<()> {
    let (show, project) = load(path)?;
    println!("BlinderKitten {}", show.version.as_deref().unwrap_or("?"));
    println!("\nInterfaces:");
    for i in &show.interfaces {
        println!("  {:<12} /{:<12} {:?}", i.name, i.short_name, i.kind);
    }
    println!("\nFixture types:");
    for t in &show.fixture_types {
        println!("  {:<36} {:<13} {:>3} ch, {} cells", t.name, t.classify().label(), t.footprint(), t.cells().len());
    }
    println!("\nFixtures:");
    println!("  {:>4}  {:<14} {:<34} {:<10} {:>4}  position", "ID", "Name", "Type", "Universe", "Addr");
    for f in &show.fixtures {
        let ty = show.fixture_type(f).map_or("?", |t| t.name.as_str());
        let (uni, addr) = match routing::primary_patch(&show, &project.artnet, &f.patches) {
            Some((p, pa)) => (routing::format_port_address(pa), p.address.to_string()),
            None => ("-".into(), "-".into()),
        };
        let pos = if f.has_placement { format!("{:?} rot {:?}", f.position, f.rotation) } else { "(not placed)".into() };
        println!("  {:>4}  {:<14} {:<34} {:<10} {:>4}  {}", f.id, f.name, ty, uni, addr, pos);
    }
    if let Some(c) = show.conductor() {
        println!("\nMain conductor: {} - {} ({} cues)", c.id, c.name, c.cues.len());
        for cue in &c.cues {
            let chapter = if cue.chapter.is_empty() { String::new() } else { format!("  [chapter: {}]", cue.chapter) };
            println!("  {:>6}  {}{}", cue.id, cue.name, chapter);
        }
    }
    for w in &show.warnings {
        eprintln!("warning: {w}");
    }
    Ok(())
}

fn monitor(show_path: Option<PathBuf>, bind: Ipv4Addr, port: u16) -> Result<()> {
    let loaded = show_path.as_ref().map(load).transpose()?;
    let rx = bk_artnet::Receiver::start(bind, port).with_context(|| format!("cannot listen on {bind}:{port}"))?;
    println!("Listening for Art-Net on {} (Ctrl+C to stop)", rx.local_addr);
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let snapshot = rx.store.snapshot();
        let mut lines = vec![format!("--- {} universe(s)", snapshot.len())];
        let mut keys: Vec<_> = snapshot.keys().copied().collect();
        keys.sort_unstable();
        for pa in keys {
            let u = &snapshot[&pa];
            let active = u.data.iter().filter(|&&b| b > 0).count();
            lines.push(format!(
                "  {:<8} from {:<21} {:>6} pkts, {:>3} ch > 0, last {:>4} ms ago",
                routing::format_port_address(pa),
                u.source,
                u.packets,
                active,
                u.last_seen.elapsed().as_millis()
            ));
        }
        if let Some((show, project)) = &loaded {
            for f in &show.fixtures {
                let (Some(ty), Some((patch, pa))) = (show.fixture_type(f), routing::primary_patch(show, &project.artnet, &f.patches)) else {
                    continue;
                };
                let Some(u) = snapshot.get(&pa) else { continue };
                let state = ty.decode(patch.address, &u.data);
                let class = ty.classify();
                let cells = ty.cells();
                let out = if cells.is_empty() {
                    light::compute(&state, None, class)
                } else {
                    light::mix(&cells.iter().map(|&c| light::compute(&state, Some(c), class)).collect::<Vec<_>>())
                };
                let mut extra = String::new();
                for (attr, range) in [(Attribute::Pan, [0.0, 360.0]), (Attribute::Tilt, [0.0, 270.0]), (Attribute::Zoom, [10.0, 40.0])] {
                    if let Some(v) = state.get(cells.first().copied(), &attr) {
                        extra += &format!(" {attr:?} {:.1}°", v.physical(range));
                    }
                }
                lines.push(format!(
                    "  [{:>3}] {:<14} int {:>3.0}% rgb {:.2} {:.2} {:.2}{}{}",
                    f.id,
                    f.name,
                    out.intensity * 100.0,
                    out.color[0],
                    out.color[1],
                    out.color[2],
                    extra,
                    out.strobe_hz.map(|h| format!(" strobe {h:.1} Hz")).unwrap_or_default()
                ));
            }
        }
        println!("{}", lines.join("\n"));
    }
}

/// Writes a normalized value into a universe the way BK's SubFixtureChannel does.
fn write_channel(universe: &mut [u8; 512], address: u16, ch: &bk_olga::Channel, value: f32) {
    let v = if ch.invert { 1.0 - value } else { value }.clamp(0.0, 1.0);
    let at = |offset: u16| usize::from(address) + usize::from(offset) - 2;
    match ch.fine_offset {
        Some(fine) => {
            let v16 = (v * 65535.0).round() as u16;
            if let Some(b) = universe.get_mut(at(ch.offset)) {
                *b = (v16 >> 8) as u8;
            }
            if let Some(b) = universe.get_mut(at(fine)) {
                *b = (v16 & 0xff) as u8;
            }
        }
        None => {
            if let Some(b) = universe.get_mut(at(ch.offset)) {
                *b = (v * 255.0).round() as u8;
            }
        }
    }
}

fn test_send(path: &PathBuf, target: SocketAddr, fps: f32) -> Result<()> {
    let (show, project) = load(path)?;
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    let start = Instant::now();
    let mut seq = 1u8;
    println!("Sending test patterns to {target} (Ctrl+C to stop)");
    loop {
        let t = start.elapsed().as_secs_f32();
        let mut universes: std::collections::BTreeMap<u16, [u8; 512]> = Default::default();
        for (i, f) in show.fixtures.iter().enumerate() {
            let (Some(ty), Some((patch, pa))) = (show.fixture_type(f), routing::primary_patch(&show, &project.artnet, &f.patches)) else {
                continue;
            };
            let uni = universes.entry(pa).or_insert([0; 512]);
            let phase = i as f32 * 0.7;
            for ch in &ty.channels {
                let cell = ch.sub_fixture as f32;
                let wave = |speed: f32, offset: f32| 0.5 + 0.5 * (t * speed + phase + offset).sin();
                let value = match ch.attribute {
                    Attribute::Intensity => 1.0,
                    Attribute::Pan => 0.5 + 0.15 * (t * 0.5 + phase).sin(),
                    Attribute::Tilt => 0.5 + 0.25 * (t * 0.7 + phase).cos(),
                    Attribute::Zoom => wave(0.3, 0.0),
                    Attribute::Red => wave(1.0, cell * 0.5),
                    Attribute::Green => wave(1.0, cell * 0.5 + 2.1),
                    Attribute::Blue => wave(1.0, cell * 0.5 + 4.2),
                    // Step through the wheel slots every few seconds.
                    Attribute::ColorWheel | Attribute::Gobo => (t * 0.3 + phase * 3.0).floor().rem_euclid(8.0) * 32.0 / 255.0,
                    _ => ch.default_value,
                };
                write_channel(uni, patch.address, ch, value);
            }
        }
        for (pa, data) in &universes {
            socket.send_to(&bk_artnet::encode(*pa, seq, data), target)?;
        }
        seq = seq.wrapping_add(1).max(1);
        std::thread::sleep(Duration::from_secs_f32(1.0 / fps.max(1.0)));
    }
}
