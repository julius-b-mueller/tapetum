//! 3Dconnexion SpaceMouse: the six axes of the cap drive the orbit camera.
//!
//! Read straight over HID in a background thread. On macOS the 3Dconnexion
//! driver (3DxWare) holds the device exclusively; there its client framework
//! delivers the same data instead (see [`mac`]). The device keeps reporting how
//! far the cap is pushed; the camera moves at a speed proportional to that.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;

/// Logitech (older devices) and 3Dconnexion.
const LOGITECH: u16 = 0x046d;
const THREEDCONNEXION: u16 = 0x256f;
/// Logitech-era 3Dconnexion products (SpaceMouse, SpaceNavigator, SpaceExplorer, SpacePilot, …).
const LOGITECH_PRODUCTS: [u16; 11] = [0xc603, 0xc605, 0xc606, 0xc621, 0xc623, 0xc625, 0xc626, 0xc627, 0xc628, 0xc629, 0xc62b];
/// Full deflection the devices report, roughly.
const FULL: f32 = 350.0;
/// Small deflections are hand tremor, not intent.
const DEAD_ZONE: f32 = 0.04;

/// Cap deflection, -1..1 per axis, in the device's frame: x right, y towards
/// the user, z down; rotations about those axes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Axes {
    pub translation: Vec3,
    pub rotation: Vec3,
}

#[derive(Default)]
struct Shared {
    axes: Axes,
    buttons: u32,
    device: Option<String>,
}

#[derive(Resource, Clone, Default)]
pub struct SpaceMouse(Arc<Mutex<Shared>>);

impl SpaceMouse {
    /// Starts looking for a device in the background (also later plugged-in ones).
    /// On macOS call this on the main thread: the driver delivers to its run loop.
    pub fn start() -> Self {
        let mouse = SpaceMouse::default();
        #[cfg(target_os = "macos")]
        if mac::start(mouse.0.clone()) {
            return mouse;
        }
        let shared = mouse.0.clone();
        std::thread::Builder::new()
            .name("spacemouse".into())
            .spawn(move || run(&shared))
            .ok();
        mouse
    }

    pub fn axes(&self) -> Axes {
        self.0.lock().map(|s| s.axes).unwrap_or_default()
    }

    pub fn buttons(&self) -> u32 {
        self.0.lock().map(|s| s.buttons).unwrap_or_default()
    }

    /// Tells the driver whether Tapetum wants the device (macOS; elsewhere HID is shared anyway).
    pub fn set_active(&self, active: bool) {
        #[cfg(target_os = "macos")]
        mac::set_active(active);
        #[cfg(not(target_os = "macos"))]
        let _ = active;
    }

    /// Name of the connected device.
    pub fn device(&self) -> Option<String> {
        self.0.lock().ok().and_then(|s| s.device.clone())
    }
}

fn is_spacemouse(d: &hidapi::DeviceInfo) -> bool {
    match d.vendor_id() {
        // A multi-axis controller (generic desktop page). 3Dconnexion also makes
        // mice and keyboards; only where the system tells no usage at all (some
        // Linux setups) any of its devices is tried.
        THREEDCONNEXION => d.usage_page() == 1 && d.usage() == 8 || d.usage_page() == 0 && d.usage() == 0,
        LOGITECH => LOGITECH_PRODUCTS.contains(&d.product_id()),
        _ => false,
    }
}

fn run(shared: &Mutex<Shared>) {
    let Ok(mut api) = hidapi::HidApi::new() else { return };
    loop {
        let _ = api.refresh_devices();
        let found = api.device_list().find(|d| is_spacemouse(d)).map(|d| (d.path().to_owned(), d.product_string().unwrap_or("SpaceMouse").to_string()));
        if let Some((path, name)) = found
            && let Ok(device) = api.open_path(&path)
        {
            if let Ok(mut s) = shared.lock() {
                s.device = Some(name);
            }
            read(&device, shared);
            if let Ok(mut s) = shared.lock() {
                *s = Shared::default();
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Until the device goes away.
fn read(device: &hidapi::HidDevice, shared: &Mutex<Shared>) {
    let mut buf = [0u8; 64];
    loop {
        match device.read_timeout(&mut buf, 200) {
            Ok(0) => {}
            Ok(n) => {
                if let Ok(mut s) = shared.lock() {
                    let Shared { axes, buttons, .. } = &mut *s;
                    apply_report(&buf[..n], axes, buttons);
                }
            }
            Err(_) => return,
        }
    }
}

/// Axis value from the device (or driver) to -1..1 with a dead zone.
fn normalize(v: i16) -> f32 {
    let v = v as f32 / FULL;
    if v.abs() < DEAD_ZONE { 0.0 } else { v.clamp(-1.0, 1.0) }
}

/// Report 1: translation (newer devices: translation and rotation), 2:
/// rotation, 3: buttons (bit field).
fn apply_report(r: &[u8], axes: &mut Axes, buttons: &mut u32) {
    let value = |i: usize| -> Option<f32> {
        let b = r.get(1 + 2 * i..3 + 2 * i)?;
        Some(normalize(i16::from_le_bytes([b[0], b[1]])))
    };
    let three = |from: usize| -> Option<Vec3> { Some(Vec3::new(value(from)?, value(from + 1)?, value(from + 2)?)) };
    match r.first() {
        Some(1) => {
            if let Some(t) = three(0) {
                axes.translation = t;
            }
            if let Some(rot) = three(3) {
                axes.rotation = rot;
            }
        }
        Some(2) => {
            if let Some(rot) = three(0) {
                axes.rotation = rot;
            }
        }
        Some(3) => {
            *buttons = r[1..].iter().take(4).enumerate().fold(0, |acc, (i, &b)| acc | (b as u32) << (8 * i));
        }
        _ => {}
    }
}

/// Prints all HID devices, then the raw reports of the SpaceMouse.
pub fn diagnose(seconds: u64) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    if mac::diagnose(seconds) {
        return Ok(());
    }
    let api = hidapi::HidApi::new()?;
    for d in api.device_list() {
        println!(
            "{:04x}:{:04x}  usage {:#x}/{:#x}  if {}  {}{}  {}",
            d.vendor_id(),
            d.product_id(),
            d.usage_page(),
            d.usage(),
            d.interface_number(),
            d.manufacturer_string().unwrap_or("?"),
            d.product_string().map(|p| format!(" {p}")).unwrap_or_default(),
            if is_spacemouse(d) { "<- SpaceMouse" } else { "" },
        );
    }
    let Some(d) = api.device_list().find(|d| is_spacemouse(d)) else {
        println!("Keine SpaceMouse gefunden.");
        return Ok(());
    };
    let device = api.open_path(d.path())?;
    println!("Geöffnet, lausche {seconds} s – bitte die Kappe bewegen …");
    let start = std::time::Instant::now();
    let mut buf = [0u8; 64];
    let (mut axes, mut buttons) = (Axes::default(), 0);
    while start.elapsed().as_secs() < seconds {
        let n = device.read_timeout(&mut buf, 200)?;
        if n > 0 {
            apply_report(&buf[..n], &mut axes, &mut buttons);
            println!("{:02x?}  ->  {:?} {:?} Tasten {buttons:#x}", &buf[..n], axes.translation, axes.rotation);
        }
    }
    Ok(())
}

/// The 3Dconnexion client framework (macOS, with the 3DxWare driver).
#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::sync::{Arc, Mutex, OnceLock};

    use super::{Shared, normalize};
    use crate::i18n::tr;

    const FRAMEWORK: &str = "/Library/Frameworks/3DconnexionClient.framework/3DconnexionClient";
    /// Manual client ('++++'): Tapetum switches itself on and off with its window
    /// focus ([`set_active`]); the driver's own front-app detection doesn't know
    /// an executable outside an app bundle. The wildcard '****' would take the
    /// device away from all other programs.
    const MANUAL: u32 = 0x2B2B_2B2B;
    /// Our name as a Pascal string.
    const NAME: &[u8] = b"\x07tapetum";
    const CTL_ACTIVATE: u32 = 0x3364_6163; // '3dac'
    const CTL_DEACTIVATE: u32 = 0x3364_6463; // '3ddc'
    const CTL_DEVICE_ID: u32 = 0x3364_6964; // '3did'

    type Control = unsafe extern "C" fn(u16, u32, i32, *mut i32) -> i16;

    /// Client ID and the control call, once registered.
    static CLIENT: OnceLock<(u16, Control)> = OnceLock::new();
    const MODE_TAKE_OVER: u16 = 1;
    const MASK_ALL: u32 = 0x3FFF;
    const MASK_ALL_BUTTONS: u32 = 0xFFFF_FFFF;
    const MSG_DEVICE_STATE: u32 = 0x3364_5352; // '3dSR'
    const CMD_BUTTONS: u16 = 2;
    const CMD_AXIS: u16 = 3;

    /// `ConnexionDeviceState`, packed to 2 bytes like the header.
    #[repr(C, packed(2))]
    struct DeviceState {
        version: u16,
        client: u16,
        command: u16,
        param: i16,
        value: i32,
        time: u64,
        report: [u8; 8],
        buttons8: u16,
        axis: [i16; 6],
        address: u16,
        buttons: u32,
    }

    type Message = extern "C" fn(u32, u32, *mut c_void);
    type Device = extern "C" fn(u32);

    static SHARED: OnceLock<Arc<Mutex<Shared>>> = OnceLock::new();

    extern "C" fn on_message(_product: u32, kind: u32, arg: *mut c_void) {
        if kind != MSG_DEVICE_STATE || arg.is_null() {
            return;
        }
        // SAFETY: for this message type the driver passes a `ConnexionDeviceState`.
        let state = unsafe { std::ptr::read_unaligned(arg as *const DeviceState) };
        let Some(Ok(mut s)) = SHARED.get().map(|s| s.lock()) else { return };
        match state.command {
            CMD_AXIS => {
                let a = state.axis;
                s.axes.translation = bevy::math::Vec3::new(normalize(a[0]), normalize(a[1]), normalize(a[2]));
                s.axes.rotation = bevy::math::Vec3::new(normalize(a[3]), normalize(a[4]), normalize(a[5]));
            }
            CMD_BUTTONS => s.buttons = state.buttons,
            _ => {}
        }
    }

    extern "C" fn on_added(_product: u32) {
        if let Some(Ok(mut s)) = SHARED.get().map(|s| s.lock()) {
            s.device = Some(tr("SpaceMouse (3Dconnexion-Treiber)", "SpaceMouse (3Dconnexion driver)").into());
        }
    }

    extern "C" fn on_removed(_product: u32) {
        if let Some(Ok(mut s)) = SHARED.get().map(|s| s.lock()) {
            *s = Shared::default();
        }
    }

    /// Registers with the driver; false if it isn't installed. Messages arrive
    /// on the calling thread's run loop (the main thread's, run by the window).
    pub fn start(shared: Arc<Mutex<Shared>>) -> bool {
        if SHARED.set(shared).is_err() {
            return true;
        }
        // SAFETY: loading the vendor's framework and calling its documented C API
        // with the signatures from ConnexionClientAPI.h. It stays loaded for good.
        unsafe {
            let Ok(lib) = libloading::Library::new(FRAMEWORK) else { return false };
            let lib: &'static libloading::Library = Box::leak(Box::new(lib));
            let (Ok(set_handlers), Ok(register), Ok(button_mask), Ok(control)) = (
                lib.get::<unsafe extern "C" fn(Message, Device, Device, bool) -> i16>(b"SetConnexionHandlers"),
                lib.get::<unsafe extern "C" fn(u32, *const u8, u16, u32) -> u16>(b"RegisterConnexionClient"),
                lib.get::<unsafe extern "C" fn(u16, u32)>(b"SetConnexionClientButtonMask"),
                lib.get::<Control>(b"ConnexionClientControl"),
            ) else {
                return false;
            };
            if set_handlers(on_message, on_added, on_removed, false) != 0 {
                return false;
            }
            let client = register(MANUAL, NAME.as_ptr(), MODE_TAKE_OVER, MASK_ALL);
            if client == 0 {
                return false;
            }
            button_mask(client, MASK_ALL_BUTTONS);
            let _ = CLIENT.set((client, *control));
        }
        true
    }

    /// Takes the device while Tapetum has the focus and gives it back otherwise.
    pub fn set_active(active: bool) {
        if let Some(&(client, control)) = CLIENT.get() {
            let mut result = 0;
            // SAFETY: documented control call for a manual client.
            unsafe { control(client, if active { CTL_ACTIVATE } else { CTL_DEACTIVATE }, 0, &mut result) };
        }
        if !active && let Some(Ok(mut s)) = SHARED.get().map(|s| s.lock()) {
            s.axes = super::Axes::default();
        }
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source_handled: u8) -> i32;
    }

    /// Lets the driver deliver for a while (the diagnosis has no window loop).
    fn run_loop(seconds: f64) {
        // SAFETY: plain CoreFoundation call on the current thread.
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, 0);
        }
    }

    fn fourcc(v: u32) -> String {
        v.to_be_bytes().iter().map(|&b| if b.is_ascii_graphic() { b as char } else { '.' }).collect()
    }

    extern "C" fn print_message(product: u32, kind: u32, arg: *mut c_void) {
        if kind == MSG_DEVICE_STATE && !arg.is_null() {
            // SAFETY: for this message type the driver passes a `ConnexionDeviceState`.
            let s = unsafe { std::ptr::read_unaligned(arg as *const DeviceState) };
            let (version, client, command, param, value, report, axis, buttons) = (s.version, s.client, s.command, s.param, s.value, s.report, s.axis, s.buttons);
            println!("  {product:#x} '3dSR' v{version:#x} client {client} cmd {command} param {param} value {value} report {report:02x?} axis {axis:?} buttons {buttons:#x}");
        } else {
            println!("  {product:#x} Nachricht '{}'", fourcc(kind));
        }
    }

    extern "C" fn print_added(product: u32) {
        println!("  Gerät angemeldet: {product:#x}");
    }

    extern "C" fn print_removed(product: u32) {
        println!("  Gerät abgemeldet: {product:#x}");
    }

    /// Registers in several ways and prints everything the driver sends; false
    /// without the driver.
    pub fn diagnose(seconds: u64) -> bool {
        // SAFETY: as in `start`.
        unsafe {
            let Ok(lib) = libloading::Library::new(FRAMEWORK) else {
                println!("Kein 3Dconnexion-Treiber ({FRAMEWORK}), versuche HID direkt.");
                return false;
            };
            let set_handlers = lib.get::<unsafe extern "C" fn(Message, Device, Device, bool) -> i16>(b"SetConnexionHandlers").unwrap();
            let cleanup = lib.get::<unsafe extern "C" fn()>(b"CleanupConnexionHandlers").unwrap();
            let register = lib.get::<unsafe extern "C" fn(u32, *const u8, u16, u32) -> u16>(b"RegisterConnexionClient").unwrap();
            let unregister = lib.get::<unsafe extern "C" fn(u16)>(b"UnregisterConnexionClient").unwrap();
            let button_mask = lib.get::<unsafe extern "C" fn(u16, u32)>(b"SetConnexionClientButtonMask").unwrap();
            let control = lib.get::<unsafe extern "C" fn(u16, u32, i32, *mut i32) -> i16>(b"ConnexionClientControl").unwrap();
            let phase = (seconds / 2).max(2) as f64;
            for separate_thread in [false, true] {
                println!("\n== Empfang {} ==", if separate_thread { "über eigenen Thread" } else { "über die Hauptschleife" });
                println!("SetConnexionHandlers -> {}", set_handlers(print_message, print_added, print_removed, separate_thread));
                let client = register(MANUAL, NAME.as_ptr(), MODE_TAKE_OVER, MASK_ALL);
                println!("RegisterConnexionClient -> Client {client}");
                button_mask(client, MASK_ALL_BUTTONS);
                let mut result = 0i32;
                println!("Aktivieren -> {} ", control(client, CTL_ACTIVATE, 0, &mut result));
                let err = control(client, CTL_DEVICE_ID, 0, &mut result);
                println!("Geräte-ID -> {err}, {:#010x}", result as u32);
                println!("Bitte jetzt {phase:.0} s lang die Kappe bewegen und die Tasten drücken …");
                if separate_thread {
                    std::thread::sleep(std::time::Duration::from_secs_f64(phase));
                } else {
                    run_loop(phase);
                }
                unregister(client);
                cleanup();
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: u8, values: &[i16]) -> Vec<u8> {
        std::iter::once(id).chain(values.iter().flat_map(|v| v.to_le_bytes())).collect()
    }

    #[test]
    fn reads_old_and_new_reports() {
        let (mut axes, mut buttons) = (Axes::default(), 0);
        // Older devices: translation and rotation in separate reports.
        apply_report(&report(1, &[350, -175, 5]), &mut axes, &mut buttons);
        apply_report(&report(2, &[0, 0, -350]), &mut axes, &mut buttons);
        assert_eq!(axes.translation, Vec3::new(1.0, -0.5, 0.0), "5 is inside the dead zone");
        assert_eq!(axes.rotation, Vec3::new(0.0, 0.0, -1.0));
        // Newer devices: all six in report 1.
        apply_report(&report(1, &[0, 0, 350, 175, 0, 0]), &mut axes, &mut buttons);
        assert_eq!(axes.translation, Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(axes.rotation, Vec3::new(0.5, 0.0, 0.0));
        apply_report(&[3, 0b10, 0], &mut axes, &mut buttons);
        assert_eq!(buttons, 2);
    }
}
