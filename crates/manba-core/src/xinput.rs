//! Acceso a XInput: estado del mando, vibración, batería y detección de conexión.

use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{ERROR_DEVICE_NOT_CONNECTED, ERROR_SUCCESS};
use windows_sys::Win32::UI::Input::XboxController::{
    XInputGetBatteryInformation, XInputGetState, XInputSetState, BATTERY_DEVTYPE_GAMEPAD,
    XINPUT_BATTERY_INFORMATION, XINPUT_STATE, XINPUT_VIBRATION,
};

/// XInput admite como máximo 4 mandos simultáneos.
pub const MAX_SLOTS: u32 = 4;

/// Cada cuánto se buscan mandos nuevos en las ranuras vacías.
/// Consultar una ranura vacía es caro, por eso no se hace en cada ciclo.
const RESCAN_INTERVAL: Duration = Duration::from_secs(1);

/// Botones pulsados, como máscara de bits de XInput.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Buttons(pub u16);

impl Buttons {
    pub const DPAD_UP: u16 = 0x0001;
    pub const DPAD_DOWN: u16 = 0x0002;
    pub const DPAD_LEFT: u16 = 0x0004;
    pub const DPAD_RIGHT: u16 = 0x0008;
    pub const START: u16 = 0x0010;
    pub const BACK: u16 = 0x0020;
    pub const LEFT_THUMB: u16 = 0x0040;
    pub const RIGHT_THUMB: u16 = 0x0080;
    pub const LB: u16 = 0x0100;
    pub const RB: u16 = 0x0200;
    pub const A: u16 = 0x1000;
    pub const B: u16 = 0x2000;
    pub const X: u16 = 0x4000;
    pub const Y: u16 = 0x8000;

    const NAMES: [(u16, &'static str); 14] = [
        (Self::A, "A"),
        (Self::B, "B"),
        (Self::X, "X"),
        (Self::Y, "Y"),
        (Self::LB, "LB"),
        (Self::RB, "RB"),
        (Self::BACK, "Back"),
        (Self::START, "Start"),
        (Self::LEFT_THUMB, "L3"),
        (Self::RIGHT_THUMB, "R3"),
        (Self::DPAD_UP, "Arriba"),
        (Self::DPAD_DOWN, "Abajo"),
        (Self::DPAD_LEFT, "Izquierda"),
        (Self::DPAD_RIGHT, "Derecha"),
    ];

    pub fn is_pressed(self, button: u16) -> bool {
        self.0 & button != 0
    }

    /// Nombres de los botones pulsados, en un orden fijo.
    pub fn pressed_names(self) -> impl Iterator<Item = &'static str> {
        Self::NAMES
            .into_iter()
            .filter(move |&(bit, _)| self.is_pressed(bit))
            .map(|(_, name)| name)
    }
}

/// Lectura cruda del mando en un instante.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GamepadState {
    /// Cambia cada vez que cambia algo en el mando.
    pub packet: u32,
    pub buttons: Buttons,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub left_stick: (i16, i16),
    pub right_stick: (i16, i16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XInputError {
    NotConnected,
    Other(u32),
}

impl std::fmt::Display for XInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConnected => write!(f, "mando no conectado"),
            Self::Other(code) => write!(f, "error de XInput {code}"),
        }
    }
}

impl std::error::Error for XInputError {}

fn check(code: u32) -> Result<(), XInputError> {
    match code {
        ERROR_SUCCESS => Ok(()),
        ERROR_DEVICE_NOT_CONNECTED => Err(XInputError::NotConnected),
        other => Err(XInputError::Other(other)),
    }
}

pub fn get_state(slot: u32) -> Result<GamepadState, XInputError> {
    // SAFETY: XINPUT_STATE es POD; XInput lo rellena entero si devuelve éxito.
    let mut raw: XINPUT_STATE = unsafe { std::mem::zeroed() };
    check(unsafe { XInputGetState(slot, &mut raw) })?;
    let pad = raw.Gamepad;
    Ok(GamepadState {
        packet: raw.dwPacketNumber,
        buttons: Buttons(pad.wButtons),
        left_trigger: pad.bLeftTrigger,
        right_trigger: pad.bRightTrigger,
        left_stick: (pad.sThumbLX, pad.sThumbLY),
        right_stick: (pad.sThumbRX, pad.sThumbRY),
    })
}

/// Fija la velocidad de los motores de vibración (0 = parado, 65535 = máximo).
pub fn set_vibration(slot: u32, left: u16, right: u16) -> Result<(), XInputError> {
    let mut vibration = XINPUT_VIBRATION {
        wLeftMotorSpeed: left,
        wRightMotorSpeed: right,
    };
    check(unsafe { XInputSetState(slot, &mut vibration) })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Battery {
    Wired,
    Level(BatteryLevel),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryLevel {
    Empty,
    Low,
    Medium,
    Full,
}

impl std::fmt::Display for Battery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wired => write!(f, "por cable"),
            Self::Level(BatteryLevel::Empty) => write!(f, "vacía"),
            Self::Level(BatteryLevel::Low) => write!(f, "baja"),
            Self::Level(BatteryLevel::Medium) => write!(f, "media"),
            Self::Level(BatteryLevel::Full) => write!(f, "llena"),
            Self::Unknown => write!(f, "desconocida"),
        }
    }
}

pub fn get_battery(slot: u32) -> Result<Battery, XInputError> {
    // SAFETY: estructura POD que XInput rellena si devuelve éxito.
    let mut info: XINPUT_BATTERY_INFORMATION = unsafe { std::mem::zeroed() };
    check(unsafe { XInputGetBatteryInformation(slot, BATTERY_DEVTYPE_GAMEPAD, &mut info) })?;
    // Valores de BATTERY_TYPE_* y BATTERY_LEVEL_* de xinput.h.
    Ok(match info.BatteryType {
        0x01 => Battery::Wired,
        0x02 | 0x03 => Battery::Level(match info.BatteryLevel {
            0x00 => BatteryLevel::Empty,
            0x01 => BatteryLevel::Low,
            0x02 => BatteryLevel::Medium,
            _ => BatteryLevel::Full,
        }),
        _ => Battery::Unknown,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Connected(u32),
    Disconnected(u32),
    /// Solo se emite cuando el estado cambia respecto a la lectura anterior.
    State(u32, GamepadState),
}

/// Sondea los mandos conectados y detecta conexiones y desconexiones.
#[derive(Debug, Default)]
pub struct Poller {
    last: [Option<GamepadState>; MAX_SLOTS as usize],
    last_scan: Option<Instant>,
    ignored: Option<u32>,
}

impl Poller {
    pub fn new() -> Self {
        Self::default()
    }

    /// Excluye una ranura del sondeo. Se usa para el mando virtual, que
    /// XInput ve como un mando más y no debe leerse a sí mismo.
    pub fn set_ignored(&mut self, slot: Option<u32>) {
        self.ignored = slot;
        if let Some(slot) = slot {
            self.last[slot as usize] = None;
        }
    }

    /// Ranuras con un mando conectado.
    pub fn connected(&self) -> impl Iterator<Item = u32> + '_ {
        (0..MAX_SLOTS).filter(|&slot| self.last[slot as usize].is_some())
    }

    /// Ejecuta un ciclo de sondeo y añade a `events` lo ocurrido.
    pub fn poll(&mut self, events: &mut Vec<Event>) {
        let rescan = self
            .last_scan
            .map_or(true, |at| at.elapsed() >= RESCAN_INTERVAL);
        if rescan {
            self.last_scan = Some(Instant::now());
        }

        for slot in 0..MAX_SLOTS {
            if self.ignored == Some(slot) {
                continue;
            }
            let previous = &mut self.last[slot as usize];
            if previous.is_none() && !rescan {
                continue;
            }
            match get_state(slot) {
                Ok(state) => {
                    match previous {
                        None => {
                            events.push(Event::Connected(slot));
                            events.push(Event::State(slot, state));
                        }
                        Some(old) if old.packet != state.packet => {
                            events.push(Event::State(slot, state));
                        }
                        Some(_) => {}
                    }
                    *previous = Some(state);
                }
                Err(_) => {
                    if previous.take().is_some() {
                        events.push(Event::Disconnected(slot));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressed_names_follow_mask() {
        let buttons = Buttons(Buttons::A | Buttons::RB | Buttons::DPAD_LEFT);
        let names: Vec<_> = buttons.pressed_names().collect();
        assert_eq!(names, ["A", "RB", "Izquierda"]);
    }

    #[test]
    fn no_buttons_pressed() {
        assert_eq!(Buttons::default().pressed_names().count(), 0);
    }
}
