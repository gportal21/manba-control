//! Salidas hacia el sistema: mando virtual (ViGEmBus) y teclado/ratón (SendInput).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use manba_core::keys::Key;
use manba_core::mapping::{Mapped, MouseButton, PadOutput};
use manba_core::xinput;
use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEINPUT,
};

pub enum VirtualPadError {
    /// ViGEmBus no está instalado.
    NoDriver,
    Other(String),
}

/// Mando de Xbox 360 virtual que ven los juegos.
///
/// La vibración que los juegos envían al mando virtual se reenvía al mando
/// físico desde un hilo que espera bloqueado a cada aviso del driver.
pub struct VirtualPad {
    target: Xbox360Wired<Client>,
    physical_slot: u32,
    rumble: Option<JoinHandle<()>>,
}

impl VirtualPad {
    /// `intensity` es el multiplicador de vibración del perfil, como bits de
    /// un `f32`, para poder cambiarlo al recargar el perfil.
    pub fn connect(physical_slot: u32, intensity: Arc<AtomicU32>) -> Result<Self, VirtualPadError> {
        let client = Client::connect().map_err(|e| match e {
            vigem_client::Error::BusNotFound => VirtualPadError::NoDriver,
            other => VirtualPadError::Other(format!("{other:?}")),
        })?;
        let mut target = Xbox360Wired::new(client, TargetId::XBOX360_WIRED);
        let other = |e: vigem_client::Error| VirtualPadError::Other(format!("{e:?}"));
        target.plugin().map_err(other)?;
        target.wait_ready().map_err(other)?;
        // El mando recién creado informa valores basura en los sticks, y el
        // driver descarta un primer estado todo a cero por no verlo como un
        // cambio. Un estado mínimo seguido de cero lo deja en reposo.
        let nudge = XGamepad {
            thumb_lx: 1,
            ..Default::default()
        };
        target.update(&nudge).map_err(other)?;
        target.update(&XGamepad::default()).map_err(other)?;

        // Sin vibración el mando sigue siendo útil, así que un fallo aquí no
        // impide crearlo.
        let rumble = target.request_notification().ok().map(|request| {
            request.spawn_thread(move |_, data| {
                let factor = f32::from_bits(intensity.load(Ordering::Relaxed));
                let scale = |motor: u8| (motor as f32 * 257.0 * factor).min(65535.0) as u16;
                let _ = xinput::set_vibration(
                    physical_slot,
                    scale(data.large_motor),
                    scale(data.small_motor),
                );
            })
        });

        Ok(Self {
            target,
            physical_slot,
            rumble,
        })
    }

    pub fn update(&mut self, pad: &PadOutput) -> Result<(), String> {
        let gamepad = XGamepad {
            buttons: XButtons { raw: pad.buttons },
            left_trigger: pad.left_trigger,
            right_trigger: pad.right_trigger,
            thumb_lx: pad.left_stick.0,
            thumb_ly: pad.left_stick.1,
            thumb_rx: pad.right_stick.0,
            thumb_ry: pad.right_stick.1,
        };
        self.target.update(&gamepad).map_err(|e| format!("{e:?}"))
    }
}

impl Drop for VirtualPad {
    fn drop(&mut self) {
        // Al desconectar, el driver aborta la espera y el hilo de vibración termina.
        let _ = self.target.unplug();
        if let Some(rumble) = self.rumble.take() {
            let _ = rumble.join();
        }
        let _ = xinput::set_vibration(self.physical_slot, 0, 0);
    }
}

const MOUSE_FLAGS: [(u8, u32, u32); 3] = [
    (MouseButton::LEFT, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
    (MouseButton::RIGHT, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
    (MouseButton::MIDDLE, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
];

/// Lleva la cuenta de qué teclas y botones están pulsados para enviar solo
/// los cambios, y acumula las fracciones de píxel del movimiento del cursor.
#[derive(Default)]
pub struct KeyboardMouse {
    keys: Vec<Key>,
    mouse_buttons: u8,
    remainder: (f32, f32),
}

impl KeyboardMouse {
    pub fn apply(&mut self, mapped: &Mapped, elapsed: Duration) {
        let mut inputs = Vec::new();

        for key in &self.keys {
            if !mapped.keys.contains(key) {
                inputs.push(key_input(*key, false));
            }
        }
        for key in &mapped.keys {
            if !self.keys.contains(key) {
                inputs.push(key_input(*key, true));
            }
        }
        self.keys.clone_from(&mapped.keys);

        let changed = self.mouse_buttons ^ mapped.mouse_buttons;
        for (mask, down, up) in MOUSE_FLAGS {
            if changed & mask != 0 {
                let flag = if mapped.mouse_buttons & mask != 0 { down } else { up };
                inputs.push(mouse_input(0, 0, flag));
            }
        }
        self.mouse_buttons = mapped.mouse_buttons;

        let (vx, vy) = mapped.mouse_velocity;
        if vx == 0.0 && vy == 0.0 {
            self.remainder = (0.0, 0.0);
        } else {
            let secs = elapsed.as_secs_f32();
            let dx = self.remainder.0 + vx * secs;
            let dy = self.remainder.1 + vy * secs;
            let (ix, iy) = (dx.trunc(), dy.trunc());
            self.remainder = (dx - ix, dy - iy);
            if ix != 0.0 || iy != 0.0 {
                inputs.push(mouse_input(ix as i32, iy as i32, MOUSEEVENTF_MOVE));
            }
        }

        if !inputs.is_empty() {
            unsafe {
                SendInput(
                    inputs.len() as u32,
                    inputs.as_ptr(),
                    std::mem::size_of::<INPUT>() as i32,
                );
            }
        }
    }

    /// Suelta todo lo que siga pulsado.
    pub fn release_all(&mut self) {
        self.apply(&Mapped::default(), Duration::ZERO);
    }
}

impl Drop for KeyboardMouse {
    fn drop(&mut self) {
        self.release_all();
    }
}

fn key_input(key: Key, down: bool) -> INPUT {
    let mut flags = KEYEVENTF_SCANCODE;
    if key.extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !down {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0,
                wScan: key.scancode,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_input(dx: i32, dy: i32, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
