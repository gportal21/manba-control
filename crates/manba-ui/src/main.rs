//! Manba Control: ventana de configuración.
//!
//! Edita el perfil y muestra el mando en vivo. No lee el mando directamente
//! (HidHide lo oculta para todo lo que no sea el servicio): toma el estado
//! que el servicio publica en memoria compartida.

// Sin ventana de consola en la versión final.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use manba_core::live::{LiveReader, Snapshot};
use manba_core::mapping::{self, Control, Mapper};
use manba_core::profile::{self, Profile};
use manba_core::{keys, xinput};
use serde::Serialize;
use tauri::State;

struct App {
    profile_path: PathBuf,
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
    profile_path: String,
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
        profile_path: app.profile_path.display().to_string(),
    }
}

#[tauri::command]
fn load_profile(app: State<App>) -> Result<Profile, String> {
    profile::ensure_exists(&app.profile_path).map_err(|e| e.to_string())?;
    Profile::load(&app.profile_path)
}

#[tauri::command]
fn save_profile(app: State<App>, profile: Profile) -> Result<(), String> {
    Mapper::new(&profile)?;
    profile.save(&app.profile_path)
}

#[derive(Serialize)]
struct LiveView {
    /// El servicio está en marcha y publicando.
    service: bool,
    state: Option<Snapshot>,
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
        state,
        left,
        right,
        error,
    }
}

/// Hace vibrar los mandos visibles. Con el servicio en marcha es el mando
/// virtual, y el servicio reenvía la vibración al Manba con la intensidad
/// del perfil guardado.
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
    let profile_path = profile::default_path().expect("no se encontró la carpeta APPDATA");
    tauri::Builder::default()
        .manage(App {
            profile_path,
            reader: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            options,
            load_profile,
            save_profile,
            live,
            test_vibration
        ])
        .run(tauri::generate_context!())
        .expect("no se pudo iniciar la ventana de configuración");
}
