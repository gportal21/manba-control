//! Manba Control: ventana de configuración.
//!
//! Edita los perfiles y muestra el mando en vivo. No lee el mando
//! directamente (HidHide lo oculta para todo lo que no sea el servicio):
//! toma el estado que el servicio publica en memoria compartida.

// Sin ventana de consola en la versión final.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use manba_core::live::{LiveReader, Snapshot};
use manba_core::mapping::{self, Control, Mapper};
use manba_core::profile::Profile;
use manba_core::store::{Store, DEFAULT_PROFILE};
use manba_core::{autostart, keys, process, xinput};
use serde::Serialize;
use tauri::State;

struct App {
    store: Store,
    /// Se abre al primer uso y se reintenta mientras el servicio no esté en marcha.
    reader: Mutex<Option<LiveReader>>,
}

#[derive(Serialize)]
struct ControlInfo {
    name: &'static str,
    /// Máscara del botón en el estado del mando; 0 en los gatillos.
    mask: u16,
    /// "LT" o "RT" para los gatillos.
    trigger: Option<&'static str>,
}

#[derive(Serialize)]
struct Options {
    controls: Vec<ControlInfo>,
    keys: Vec<&'static str>,
    profiles_dir: String,
    default_profile: &'static str,
}

#[tauri::command]
fn options(app: State<App>) -> Options {
    let controls = mapping::controls()
        .map(|(name, control)| match control {
            Control::Button(mask) => ControlInfo { name, mask, trigger: None },
            Control::LeftTrigger => ControlInfo { name, mask: 0, trigger: Some("LT") },
            Control::RightTrigger => ControlInfo { name, mask: 0, trigger: Some("RT") },
        })
        .collect();
    Options {
        controls,
        keys: keys::key_names().collect(),
        profiles_dir: app.store.profiles_dir().display().to_string(),
        default_profile: DEFAULT_PROFILE,
    }
}

#[derive(Serialize)]
struct ProfilesInfo {
    names: Vec<String>,
    base: String,
    shortcuts: bool,
}

#[tauri::command]
fn profiles(app: State<App>) -> ProfilesInfo {
    let config = app.store.load_config();
    ProfilesInfo {
        names: app.store.list(),
        base: config.base_profile,
        shortcuts: config.shortcuts,
    }
}

#[tauri::command]
fn load_profile(app: State<App>, name: String) -> Result<Profile, String> {
    app.store.load(&name)
}

#[tauri::command]
fn save_profile(app: State<App>, name: String, profile: Profile) -> Result<(), String> {
    Mapper::new(&profile)?;
    app.store.save(&name, &profile)
}

#[tauri::command]
fn create_profile(app: State<App>, name: String, from: Option<String>) -> Result<String, String> {
    app.store.create(&name, from.as_deref())
}

#[tauri::command]
fn rename_profile(app: State<App>, old: String, new: String) -> Result<String, String> {
    app.store.rename(&old, &new)
}

#[tauri::command]
fn delete_profile(app: State<App>, name: String) -> Result<(), String> {
    app.store.delete(&name)
}

#[tauri::command]
fn set_base(app: State<App>, name: String) -> Result<(), String> {
    let mut config = app.store.load_config();
    config.base_profile = name;
    app.store.save_config(&config)
}

#[tauri::command]
fn set_shortcuts(app: State<App>, enabled: bool) -> Result<(), String> {
    let mut config = app.store.load_config();
    config.shortcuts = enabled;
    app.store.save_config(&config)
}

/// Programas abiertos, para sugerir juegos.
#[tauri::command]
fn running_apps() -> Vec<String> {
    process::visible_apps()
}

#[derive(Serialize)]
struct LiveView {
    /// El servicio está en marcha y publicando.
    service: bool,
    state: Option<Snapshot>,
    /// Perfil que el servicio está aplicando ahora.
    active_profile: Option<String>,
    /// Sticks procesados con el perfil que se está editando (aún sin guardar).
    left: Option<(f32, f32)>,
    right: Option<(f32, f32)>,
    /// Error del perfil que se está editando.
    error: Option<String>,
}

#[tauri::command]
fn live(app: State<App>, profile: Profile) -> LiveView {
    let state = {
        let mut reader = app.reader.lock().unwrap();
        if reader.is_none() {
            *reader = LiveReader::open();
        }
        reader.as_ref().and_then(LiveReader::read)
    };

    let valid = profile.validate();
    let error = valid.clone().err().or_else(|| Mapper::new(&profile).err());
    let (left, right) = match &state {
        Some(s) if s.connected && valid.is_ok() => (
            Some(mapping::process_stick((s.raw_left[0], s.raw_left[1]), &profile.left_stick)),
            Some(mapping::process_stick((s.raw_right[0], s.raw_right[1]), &profile.right_stick)),
        ),
        _ => (None, None),
    };

    LiveView {
        service: state.is_some(),
        active_profile: state.as_ref().map(Snapshot::active_profile),
        state,
        left,
        right,
        error,
    }
}

/// El servicio se distribuye junto a la ventana.
fn service_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(exe.with_file_name("manba-service.exe"))
}

#[tauri::command]
fn autostart_enabled() -> bool {
    autostart::is_enabled()
}

/// Devuelve el estado real tras el cambio.
#[tauri::command]
fn set_autostart(enabled: bool) -> Result<bool, String> {
    if enabled {
        autostart::enable(&service_exe()?)?;
    } else {
        autostart::disable()?;
    }
    Ok(autostart::is_enabled())
}

/// Hace vibrar los mandos visibles. Con el servicio en marcha es el mando
/// virtual, y el servicio reenvía la vibración al Manba con la intensidad
/// del perfil activo.
#[tauri::command]
fn test_vibration() {
    thread::spawn(|| {
        let slots: Vec<u32> = (0..xinput::MAX_SLOTS)
            .filter(|&slot| xinput::get_state(slot).is_ok())
            .collect();
        for &slot in &slots {
            let _ = xinput::set_vibration(slot, 40_000, 40_000);
        }
        thread::sleep(Duration::from_millis(400));
        for &slot in &slots {
            let _ = xinput::set_vibration(slot, 0, 0);
        }
    });
}

fn main() {
    let store = Store::from_appdata().expect("no se encontró la carpeta APPDATA");
    let _ = store.init();
    tauri::Builder::default()
        .manage(App {
            store,
            reader: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            options,
            profiles,
            load_profile,
            save_profile,
            create_profile,
            rename_profile,
            delete_profile,
            set_base,
            set_shortcuts,
            running_apps,
            live,
            test_vibration,
            autostart_enabled,
            set_autostart
        ])
        .run(tauri::generate_context!())
        .expect("no se pudo iniciar la ventana de configuración");
}
