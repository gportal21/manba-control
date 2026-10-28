//! Manba Control: servicio en la bandeja del sistema.
//!
//! El hilo principal solo atiende la bandeja (mensajes de Windows); todo el
//! trabajo con el mando ocurre en el hilo del motor.

// Sin ventana de consola en la versión final.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;
mod output;

use std::path::Path;
use std::process::Command;
use std::ptr::{null, null_mut};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

use manba_core::profile;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentThreadId};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PeekMessageW, PostQuitMessage, PostThreadMessageW,
    TranslateMessage, MSG, PM_NOREMOVE, WM_APP,
};

use engine::{PadStatus, Shared, Status};

/// Mensaje que envía el motor al hilo principal cuando cambia el estado.
const WM_STATUS: u32 = WM_APP + 1;

fn main() {
    let Some(_instance) = SingleInstance::acquire() else {
        return;
    };
    let Some(profile_path) = profile::default_path() else {
        return;
    };
    let _ = profile::ensure_exists(&profile_path);

    // Crea la cola de mensajes antes de que el motor pueda escribir en ella.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE) };
    let main_thread = unsafe { GetCurrentThreadId() };

    let shared = Arc::new(Shared::new());
    let engine = {
        let shared = Arc::clone(&shared);
        let path = profile_path.clone();
        thread::Builder::new()
            .name("manba-engine".into())
            .spawn(move || {
                engine::run(&shared, &path, || unsafe {
                    PostThreadMessageW(main_thread, WM_STATUS, 0, 0);
                })
            })
            .expect("no se pudo iniciar el motor")
    };

    let tray = Tray::new();
    tray.show(&shared.status.lock().unwrap());

    while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        if msg.message == WM_STATUS && msg.hwnd.is_null() {
            tray.show(&shared.status.lock().unwrap());
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            tray.handle(&event, &shared, &profile_path);
        }
    }

    shared.stop.store(true, Ordering::Relaxed);
    let _ = engine.join();
}

struct Tray {
    icon: TrayIcon,
    status: MenuItem,
    profile_status: MenuItem,
    enabled: CheckMenuItem,
    reload: MenuItem,
    edit: MenuItem,
    quit: MenuItem,
}

impl Tray {
    fn new() -> Self {
        let status = MenuItem::new("Esperando mando", false, None);
        let profile_status = MenuItem::new("Perfil cargado", false, None);
        let enabled = CheckMenuItem::new("Perfil activo", true, true, None);
        let reload = MenuItem::new("Recargar perfil", true, None);
        let edit = MenuItem::new("Editar perfil", true, None);
        let quit = MenuItem::new("Salir", true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &status,
            &profile_status,
            &PredefinedMenuItem::separator(),
            &enabled,
            &reload,
            &edit,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .expect("no se pudo crear el menú");

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .with_tooltip("Manba Control")
            .with_icon(make_icon(false))
            .build()
            .expect("no se pudo crear el icono de la bandeja");

        Self {
            icon,
            status,
            profile_status,
            enabled,
            reload,
            edit,
            quit,
        }
    }

    fn show(&self, status: &Status) {
        let controller = match status.controller {
            Some(slot) => format!("Mando conectado ({})", slot + 1),
            None => "Esperando mando".to_string(),
        };
        let output = match &status.virtual_pad {
            PadStatus::Idle => String::new(),
            PadStatus::Active => " - mando virtual activo".to_string(),
            PadStatus::NoDriver => " - sin ViGEmBus: solo teclado y ratón".to_string(),
            PadStatus::Error(e) => format!(" - error del mando virtual: {e}"),
        };
        let text = format!("{controller}{output}");
        self.status.set_text(truncate(&text, 90));

        match &status.profile_error {
            Some(e) => self.profile_status.set_text(truncate(&format!("Error en el perfil: {e}"), 90)),
            None => self.profile_status.set_text("Perfil cargado"),
        }

        // El tooltip de la bandeja admite como máximo 127 caracteres.
        let _ = self
            .icon
            .set_tooltip(Some(truncate(&format!("Manba Control\n{text}"), 120)));
        let _ = self.icon.set_icon(Some(make_icon(status.controller.is_some())));
    }

    fn handle(&self, event: &MenuEvent, shared: &Shared, profile_path: &Path) {
        if event.id == *self.enabled.id() {
            shared
                .enabled
                .store(self.enabled.is_checked(), Ordering::Relaxed);
        } else if event.id == *self.reload.id() {
            shared.reload.store(true, Ordering::Relaxed);
        } else if event.id == *self.edit.id() {
            let _ = Command::new("notepad.exe").arg(profile_path).spawn();
        } else if event.id == *self.quit.id() {
            unsafe { PostQuitMessage(0) };
        }
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text.to_string(),
    }
}

/// Icono dibujado en código: un anillo con un punto central, verde con el
/// mando conectado y gris sin él.
fn make_icon(connected: bool) -> Icon {
    const SIZE: u32 = 32;
    const CENTER: f32 = SIZE as f32 / 2.0;
    let [r, g, b] = if connected {
        [0x3D, 0xC9, 0x7A]
    } else {
        [0x8A, 0x8F, 0x98]
    };

    // Cobertura con medio píxel de suavizado en cada borde.
    let band = |d: f32, inner: f32, outer: f32| ((d - inner + 0.5).min(outer - d + 0.5)).clamp(0.0, 1.0);

    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = (x as f32 + 0.5 - CENTER).hypot(y as f32 + 0.5 - CENTER);
            let alpha = band(d, 11.0, 15.0).max(band(d, -1.0, 5.5));
            rgba.extend_from_slice(&[r, g, b, (alpha * 255.0) as u8]);
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("icono válido")
}

/// Impide abrir dos instancias, que crearían dos mandos virtuales.
struct SingleInstance(HANDLE);

impl SingleInstance {
    fn acquire() -> Option<Self> {
        let name: Vec<u16> = "Local\\ManbaControl\0".encode_utf16().collect();
        let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return None;
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            return None;
        }
        Some(Self(handle))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
