//! Motor de mapeo: convierte la lectura del mando en lo que se envía al
//! sistema (mando virtual, teclas y ratón) según el perfil.
//!
//! No toca el sistema operativo, así que se puede probar sin hardware.

use crate::deadzone;
use crate::keys::{self, Key};
use crate::profile::{Profile, StickConfig, StickMode};
use crate::xinput::{Buttons, GamepadState};

/// Un control del mando que puede ser origen o destino de una reasignación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Button(u16),
    LeftTrigger,
    RightTrigger,
}

const CONTROLS: [(&str, Control); 16] = [
    ("A", Control::Button(Buttons::A)),
    ("B", Control::Button(Buttons::B)),
    ("X", Control::Button(Buttons::X)),
    ("Y", Control::Button(Buttons::Y)),
    ("LB", Control::Button(Buttons::LB)),
    ("RB", Control::Button(Buttons::RB)),
    ("LT", Control::LeftTrigger),
    ("RT", Control::RightTrigger),
    ("Back", Control::Button(Buttons::BACK)),
    ("Start", Control::Button(Buttons::START)),
    ("L3", Control::Button(Buttons::LEFT_THUMB)),
    ("R3", Control::Button(Buttons::RIGHT_THUMB)),
    ("Arriba", Control::Button(Buttons::DPAD_UP)),
    ("Abajo", Control::Button(Buttons::DPAD_DOWN)),
    ("Izquierda", Control::Button(Buttons::DPAD_LEFT)),
    ("Derecha", Control::Button(Buttons::DPAD_RIGHT)),
];

/// Controles reasignables, con su nombre en el perfil.
pub fn controls() -> impl Iterator<Item = (&'static str, Control)> {
    CONTROLS.into_iter()
}

fn control_index(name: &str) -> Option<usize> {
    let name = name.trim();
    CONTROLS
        .iter()
        .position(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    pub const LEFT: u8 = 0b001;
    pub const RIGHT: u8 = 0b010;
    pub const MIDDLE: u8 = 0b100;

    fn mask(self) -> u8 {
        match self {
            Self::Left => Self::LEFT,
            Self::Right => Self::RIGHT,
            Self::Middle => Self::MIDDLE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Pad(Control),
    Key(Key),
    Mouse(MouseButton),
}

fn parse_action(text: &str) -> Result<Action, String> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    if lower == "ninguno" {
        return Ok(Action::None);
    }
    if let Some(name) = lower.strip_prefix("tecla:") {
        return keys::parse_key(name).map(Action::Key).ok_or_else(|| {
            let valid: Vec<_> = keys::key_names().collect();
            format!("tecla desconocida '{name}'. Válidas: {}", valid.join(", "))
        });
    }
    if let Some(name) = lower.strip_prefix("raton:") {
        return match name.trim() {
            "izquierdo" => Ok(Action::Mouse(MouseButton::Left)),
            "derecho" => Ok(Action::Mouse(MouseButton::Right)),
            "medio" => Ok(Action::Mouse(MouseButton::Middle)),
            other => Err(format!(
                "botón de ratón desconocido '{other}' (izquierdo, derecho o medio)"
            )),
        };
    }
    control_index(text)
        .map(|i| Action::Pad(CONTROLS[i].1))
        .ok_or_else(|| format!("acción desconocida '{text}'"))
}

/// Estado que se envía al mando virtual.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PadOutput {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub left_stick: (i16, i16),
    pub right_stick: (i16, i16),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mapped {
    pub pad: PadOutput,
    /// Teclas que deben estar pulsadas.
    pub keys: Vec<Key>,
    /// Máscara de `MouseButton::LEFT | RIGHT | MIDDLE`.
    pub mouse_buttons: u8,
    /// Velocidad del cursor en píxeles por segundo (y positiva hacia abajo).
    pub mouse_velocity: (f32, f32),
}

#[derive(Debug, Clone)]
pub struct Mapper {
    bindings: [Action; CONTROLS.len()],
    left_stick: StickConfig,
    right_stick: StickConfig,
    trigger_threshold: u8,
}

impl Default for Mapper {
    /// Sin reasignaciones: el mando virtual repite al físico.
    fn default() -> Self {
        Self::new(&Profile::default()).expect("el perfil por defecto es válido")
    }
}

impl Mapper {
    /// Compila el perfil. Devuelve todos los errores de reasignación juntos.
    pub fn new(profile: &Profile) -> Result<Self, String> {
        let mut bindings = CONTROLS.map(|(_, control)| Action::Pad(control));
        let mut errors = Vec::new();
        for (source, action) in &profile.buttons {
            let Some(index) = control_index(source) else {
                errors.push(format!("control desconocido '{source}'"));
                continue;
            };
            match parse_action(action) {
                Ok(action) => bindings[index] = action,
                Err(e) => errors.push(format!("{source}: {e}")),
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        Ok(Self {
            bindings,
            left_stick: profile.left_stick.clone(),
            right_stick: profile.right_stick.clone(),
            trigger_threshold: profile.triggers.threshold,
        })
    }

    pub fn map(&self, state: &GamepadState) -> Mapped {
        let mut out = Mapped::default();

        for (&(_, control), &action) in CONTROLS.iter().zip(&self.bindings) {
            // Un gatillo sin reasignar conserva su recorrido analógico.
            if action == Action::Pad(control) {
                if let Some(raw) = self.trigger_value(control, state) {
                    let value = deadzone::trigger(raw, self.trigger_threshold);
                    set_trigger(&mut out.pad, control, (value * 255.0).round() as u8);
                    continue;
                }
            }
            if !self.is_pressed(control, state) {
                continue;
            }
            match action {
                Action::None => {}
                Action::Pad(Control::Button(mask)) => out.pad.buttons |= mask,
                Action::Pad(trigger) => set_trigger(&mut out.pad, trigger, u8::MAX),
                Action::Key(key) => {
                    if !out.keys.contains(&key) {
                        out.keys.push(key);
                    }
                }
                Action::Mouse(button) => out.mouse_buttons |= button.mask(),
            }
        }

        for (raw, config, target) in [
            (state.left_stick, &self.left_stick, &mut out.pad.left_stick),
            (state.right_stick, &self.right_stick, &mut out.pad.right_stick),
        ] {
            let (x, y) = process_stick(raw, config);
            match config.mode {
                StickMode::Stick => *target = (to_axis(x), to_axis(y)),
                StickMode::Mouse => {
                    // En pantalla la y crece hacia abajo.
                    out.mouse_velocity.0 += x * config.mouse_speed;
                    out.mouse_velocity.1 -= y * config.mouse_speed;
                }
                StickMode::Disabled => {}
            }
        }

        out
    }

    fn trigger_value(&self, control: Control, state: &GamepadState) -> Option<u8> {
        match control {
            Control::LeftTrigger => Some(state.left_trigger),
            Control::RightTrigger => Some(state.right_trigger),
            Control::Button(_) => None,
        }
    }

    fn is_pressed(&self, control: Control, state: &GamepadState) -> bool {
        match self.trigger_value(control, state) {
            Some(value) => value > self.trigger_threshold,
            None => match control {
                Control::Button(mask) => state.buttons.is_pressed(mask),
                _ => unreachable!(),
            },
        }
    }
}

fn set_trigger(pad: &mut PadOutput, trigger: Control, value: u8) {
    let slot = match trigger {
        Control::LeftTrigger => &mut pad.left_trigger,
        Control::RightTrigger => &mut pad.right_trigger,
        Control::Button(_) => return,
    };
    *slot = (*slot).max(value);
}

/// Aplica zona muerta, curva e inversión a un stick crudo. Devuelve -1..1.
pub fn process_stick(raw: (i16, i16), config: &StickConfig) -> (f32, f32) {
    let (x, y) = deadzone::radial(raw.0, raw.1, config.deadzone, config.outer);
    let (x, y) = deadzone::curve(x, y, config.curve);
    (x, if config.invert_y { -y } else { y })
}

fn to_axis(value: f32) -> i16 {
    (value.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapper(toml: &str) -> Mapper {
        Mapper::new(&Profile::parse(toml).unwrap()).unwrap()
    }

    fn pressed(buttons: u16) -> GamepadState {
        GamepadState {
            buttons: Buttons(buttons),
            ..Default::default()
        }
    }

    #[test]
    fn default_passes_buttons_through() {
        let out = Mapper::default().map(&pressed(Buttons::A | Buttons::DPAD_UP));
        assert_eq!(out.pad.buttons, Buttons::A | Buttons::DPAD_UP);
        assert!(out.keys.is_empty());
    }

    #[test]
    fn swaps_buttons() {
        let m = mapper("[botones]\nA = \"B\"\nB = \"A\"\n");
        assert_eq!(m.map(&pressed(Buttons::A)).pad.buttons, Buttons::B);
        assert_eq!(m.map(&pressed(Buttons::B)).pad.buttons, Buttons::A);
    }

    #[test]
    fn button_to_key_and_mouse() {
        let m = mapper("[botones]\nLB = \"tecla:Q\"\nRB = \"raton:izquierdo\"\n");
        let out = m.map(&pressed(Buttons::LB | Buttons::RB));
        assert_eq!(out.pad.buttons, 0);
        assert_eq!(out.keys, [keys::parse_key("Q").unwrap()]);
        assert_eq!(out.mouse_buttons, MouseButton::LEFT);
    }

    #[test]
    fn disabled_button_does_nothing() {
        let m = mapper("[botones]\nBack = \"ninguno\"\n");
        assert_eq!(m.map(&pressed(Buttons::BACK)), Mapped::default());
    }

    #[test]
    fn trigger_keeps_analog_value() {
        let state = GamepadState {
            right_trigger: 255,
            left_trigger: 10,
            ..Default::default()
        };
        let out = Mapper::default().map(&state);
        assert_eq!(out.pad.right_trigger, 255);
        assert_eq!(out.pad.left_trigger, 0);
    }

    #[test]
    fn trigger_as_button_and_button_as_trigger() {
        let m = mapper("[botones]\nRT = \"tecla:Espacio\"\nRB = \"RT\"\n");
        let state = GamepadState {
            right_trigger: 200,
            buttons: Buttons(Buttons::RB),
            ..Default::default()
        };
        let out = m.map(&state);
        assert_eq!(out.keys, [keys::parse_key("Espacio").unwrap()]);
        assert_eq!(out.pad.right_trigger, 255);
    }

    #[test]
    fn stick_as_mouse() {
        let m = mapper("[stick_derecho]\nmodo = \"raton\"\nvelocidad_raton = 1000.0\n");
        let state = GamepadState {
            right_stick: (32767, 32767 / 2),
            ..Default::default()
        };
        let out = m.map(&state);
        assert_eq!(out.pad.right_stick, (0, 0));
        assert!(out.mouse_velocity.0 > 0.0);
        // Stick hacia arriba = cursor hacia arriba = y negativa.
        assert!(out.mouse_velocity.1 < 0.0);
    }

    #[test]
    fn invert_y() {
        let m = mapper("[stick_izquierdo]\ninvertir_y = true\n");
        let state = GamepadState {
            left_stick: (0, 32767),
            ..Default::default()
        };
        assert_eq!(m.map(&state).pad.left_stick, (0, -32767));
    }

    #[test]
    fn reports_every_invalid_binding() {
        let profile =
            Profile::parse("[botones]\nZ = \"A\"\nA = \"tecla:Nada\"\nB = \"raton:lateral\"\n")
                .unwrap();
        let error = Mapper::new(&profile).unwrap_err();
        assert!(error.contains("control desconocido 'Z'"));
        assert!(error.contains("tecla desconocida"));
        assert!(error.contains("botón de ratón desconocido"));
    }
}
