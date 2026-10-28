//! Motor de mapeo: convierte la lectura del mando en lo que se envía al
//! sistema (mando virtual, teclas y ratón) según el perfil.
//!
//! No toca el sistema operativo, así que se puede probar sin hardware. Guarda
//! un poco de estado entre lecturas para el turbo y las macros, que dependen
//! de cuándo se pulsó cada control.

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

const COUNT: usize = CONTROLS.len();

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
    /// Índice de la macro en el `Mapper`.
    Macro(usize),
}

/// `macros` son los nombres de las macros del perfil, para resolver `macro:<nombre>`.
fn parse_action(text: &str, macros: &[String]) -> Result<Action, String> {
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
    if lower.starts_with("macro:") {
        let name = text["macro:".len()..].trim();
        return macros
            .iter()
            .position(|m| m.eq_ignore_ascii_case(name))
            .map(Action::Macro)
            .ok_or_else(|| format!("no existe la macro '{name}'"));
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

impl Mapped {
    fn apply(&mut self, action: Action) {
        match action {
            Action::None | Action::Macro(_) => {}
            Action::Pad(Control::Button(mask)) => self.pad.buttons |= mask,
            Action::Pad(trigger) => set_trigger(&mut self.pad, trigger, u8::MAX),
            Action::Key(key) => {
                if !self.keys.contains(&key) {
                    self.keys.push(key);
                }
            }
            Action::Mouse(button) => self.mouse_buttons |= button.mask(),
        }
    }
}

#[derive(Debug, Clone)]
struct MacroStep {
    actions: Vec<Action>,
    duration_ms: u64,
}

#[derive(Debug, Clone)]
pub struct Mapper {
    bindings: [Action; COUNT],
    /// Periodo del turbo en milisegundos, por control.
    turbo: [Option<u64>; COUNT],
    macros: Vec<Vec<MacroStep>>,
    left_stick: StickConfig,
    right_stick: StickConfig,
    trigger_threshold: u8,
    // Estado entre lecturas.
    was_pressed: [bool; COUNT],
    pressed_at: [u64; COUNT],
    /// Macros en ejecución: (índice, momento de inicio).
    running: Vec<(usize, u64)>,
}

impl Default for Mapper {
    /// Sin reasignaciones: el mando virtual repite al físico.
    fn default() -> Self {
        Self::new(&Profile::default()).expect("el perfil por defecto es válido")
    }
}

impl Mapper {
    /// Compila el perfil. Devuelve todos los errores juntos.
    pub fn new(profile: &Profile) -> Result<Self, String> {
        let mut errors = Vec::new();

        let macro_names: Vec<String> = profile.macros.keys().map(|n| n.trim().to_string()).collect();
        let mut macros = Vec::new();
        for (name, definition) in &profile.macros {
            let mut steps = Vec::new();
            for step in &definition.steps {
                let mut actions = Vec::new();
                for text in &step.press {
                    match parse_action(text, &[]) {
                        Ok(Action::None) => {}
                        Ok(action) => actions.push(action),
                        Err(e) if text.trim().to_ascii_lowercase().starts_with("macro:") => {
                            errors.push(format!("macro {name}: una macro no puede lanzar otra ({e})"))
                        }
                        Err(e) => errors.push(format!("macro {name}: {e}")),
                    }
                }
                steps.push(MacroStep {
                    actions,
                    duration_ms: u64::from(step.duration_ms),
                });
            }
            macros.push(steps);
        }

        let mut bindings = CONTROLS.map(|(_, control)| Action::Pad(control));
        for (source, action) in &profile.buttons {
            let Some(index) = control_index(source) else {
                errors.push(format!("control desconocido '{source}'"));
                continue;
            };
            match parse_action(action, &macro_names) {
                Ok(action) => bindings[index] = action,
                Err(e) => errors.push(format!("{source}: {e}")),
            }
        }

        let mut turbo = [None; COUNT];
        for (source, hz) in &profile.turbo {
            match control_index(source) {
                Some(index) => turbo[index] = Some(((1000.0 / hz).round() as u64).max(2)),
                None => errors.push(format!("turbo: control desconocido '{source}'")),
            }
        }

        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        Ok(Self {
            bindings,
            turbo,
            macros,
            left_stick: profile.left_stick.clone(),
            right_stick: profile.right_stick.clone(),
            trigger_threshold: profile.triggers.threshold,
            was_pressed: [false; COUNT],
            pressed_at: [0; COUNT],
            running: Vec::new(),
        })
    }

    /// Olvida pulsaciones y detiene las macros. Se usa al activar el perfil.
    pub fn reset(&mut self) {
        self.was_pressed = [false; COUNT];
        self.running.clear();
    }

    /// `now_ms` es un reloj en milisegundos que solo avanza; el turbo y las
    /// macros se miden con él.
    pub fn map(&mut self, state: &GamepadState, now_ms: u64) -> Mapped {
        let mut out = Mapped::default();

        for index in 0..COUNT {
            let control = CONTROLS[index].1;
            let action = self.bindings[index];
            let pressed = self.is_pressed(control, state);
            let just_pressed = pressed && !self.was_pressed[index];
            self.was_pressed[index] = pressed;
            if just_pressed {
                self.pressed_at[index] = now_ms;
            }

            // Un gatillo sin reasignar ni turbo conserva su recorrido analógico.
            if action == Action::Pad(control) && self.turbo[index].is_none() {
                if let Some(raw) = trigger_value(control, state) {
                    let value = deadzone::trigger(raw, self.trigger_threshold);
                    set_trigger(&mut out.pad, control, (value * 255.0).round() as u8);
                    continue;
                }
            }
            if !pressed {
                continue;
            }
            if let Action::Macro(macro_index) = action {
                // Se lanza al pulsar y llega hasta el final aunque se suelte.
                if just_pressed && !self.running.iter().any(|&(m, _)| m == macro_index) {
                    self.running.push((macro_index, now_ms));
                }
                continue;
            }
            if let Some(period) = self.turbo[index] {
                // Pulsado la primera mitad de cada periodo, suelto la segunda.
                let phase = now_ms.saturating_sub(self.pressed_at[index]) % period;
                if phase >= period / 2 {
                    continue;
                }
            }
            out.apply(action);
        }

        let macros = &self.macros;
        self.running.retain(|&(macro_index, started)| {
            let mut elapsed = now_ms.saturating_sub(started);
            for step in &macros[macro_index] {
                if elapsed < step.duration_ms {
                    for &action in &step.actions {
                        out.apply(action);
                    }
                    return true;
                }
                elapsed -= step.duration_ms;
            }
            false
        });

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

    fn is_pressed(&self, control: Control, state: &GamepadState) -> bool {
        match control {
            Control::Button(mask) => state.buttons.is_pressed(mask),
            trigger => trigger_value(trigger, state).is_some_and(|v| v > self.trigger_threshold),
        }
    }
}

fn trigger_value(control: Control, state: &GamepadState) -> Option<u8> {
    match control {
        Control::LeftTrigger => Some(state.left_trigger),
        Control::RightTrigger => Some(state.right_trigger),
        Control::Button(_) => None,
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
        let out = Mapper::default().map(&pressed(Buttons::A | Buttons::DPAD_UP), 0);
        assert_eq!(out.pad.buttons, Buttons::A | Buttons::DPAD_UP);
        assert!(out.keys.is_empty());
    }

    #[test]
    fn swaps_buttons() {
        let mut m = mapper("[botones]\nA = \"B\"\nB = \"A\"\n");
        assert_eq!(m.map(&pressed(Buttons::A), 0).pad.buttons, Buttons::B);
        assert_eq!(m.map(&pressed(Buttons::B), 1).pad.buttons, Buttons::A);
    }

    #[test]
    fn button_to_key_and_mouse() {
        let mut m = mapper("[botones]\nLB = \"tecla:Q\"\nRB = \"raton:izquierdo\"\n");
        let out = m.map(&pressed(Buttons::LB | Buttons::RB), 0);
        assert_eq!(out.pad.buttons, 0);
        assert_eq!(out.keys, [keys::parse_key("Q").unwrap()]);
        assert_eq!(out.mouse_buttons, MouseButton::LEFT);
    }

    #[test]
    fn disabled_button_does_nothing() {
        let mut m = mapper("[botones]\nBack = \"ninguno\"\n");
        assert_eq!(m.map(&pressed(Buttons::BACK), 0), Mapped::default());
    }

    #[test]
    fn trigger_keeps_analog_value() {
        let state = GamepadState {
            right_trigger: 255,
            left_trigger: 10,
            ..Default::default()
        };
        let out = Mapper::default().map(&state, 0);
        assert_eq!(out.pad.right_trigger, 255);
        assert_eq!(out.pad.left_trigger, 0);
    }

    #[test]
    fn trigger_as_button_and_button_as_trigger() {
        let mut m = mapper("[botones]\nRT = \"tecla:Espacio\"\nRB = \"RT\"\n");
        let state = GamepadState {
            right_trigger: 200,
            buttons: Buttons(Buttons::RB),
            ..Default::default()
        };
        let out = m.map(&state, 0);
        assert_eq!(out.keys, [keys::parse_key("Espacio").unwrap()]);
        assert_eq!(out.pad.right_trigger, 255);
    }

    #[test]
    fn stick_as_mouse() {
        let mut m = mapper("[stick_derecho]\nmodo = \"raton\"\nvelocidad_raton = 1000.0\n");
        let state = GamepadState {
            right_stick: (32767, 32767 / 2),
            ..Default::default()
        };
        let out = m.map(&state, 0);
        assert_eq!(out.pad.right_stick, (0, 0));
        assert!(out.mouse_velocity.0 > 0.0);
        // Stick hacia arriba = cursor hacia arriba = y negativa.
        assert!(out.mouse_velocity.1 < 0.0);
    }

    #[test]
    fn invert_y() {
        let mut m = mapper("[stick_izquierdo]\ninvertir_y = true\n");
        let state = GamepadState {
            left_stick: (0, 32767),
            ..Default::default()
        };
        assert_eq!(m.map(&state, 0).pad.left_stick, (0, -32767));
    }

    #[test]
    fn reports_every_invalid_binding() {
        let profile =
            Profile::parse("[botones]\nZ = \"A\"\nA = \"tecla:Nada\"\nB = \"raton:lateral\"\nX = \"macro:nada\"\n")
                .unwrap();
        let error = Mapper::new(&profile).unwrap_err();
        assert!(error.contains("control desconocido 'Z'"));
        assert!(error.contains("tecla desconocida"));
        assert!(error.contains("botón de ratón desconocido"));
        assert!(error.contains("no existe la macro 'nada'"));
    }

    #[test]
    fn turbo_toggles_while_held() {
        // 10 pulsaciones por segundo: 50 ms pulsado, 50 ms suelto.
        let mut m = mapper("[turbo]\nA = 10\n");
        let held = pressed(Buttons::A);
        let at = |m: &mut Mapper, t| m.map(&held, t).pad.buttons & Buttons::A != 0;
        assert!(at(&mut m, 1000));
        assert!(at(&mut m, 1049));
        assert!(!at(&mut m, 1050));
        assert!(!at(&mut m, 1099));
        assert!(at(&mut m, 1100));
        // Al soltar no queda nada pulsado.
        assert_eq!(m.map(&pressed(0), 1110).pad.buttons, 0);
    }

    #[test]
    fn turbo_works_with_remapped_key() {
        let mut m = mapper("[botones]\nRB = \"tecla:E\"\n[turbo]\nRB = 20\n");
        let held = pressed(Buttons::RB);
        assert_eq!(m.map(&held, 0).keys.len(), 1);
        assert!(m.map(&held, 30).keys.is_empty());
    }

    #[test]
    fn macro_runs_its_steps_once() {
        let mut m = mapper(
            "[botones]\nY = \"macro:combo\"\n\
             [macros.combo]\npasos = [\n\
             { pulsar = [\"A\"], ms = 60 },\n\
             { pulsar = [], ms = 40 },\n\
             { pulsar = [\"B\", \"tecla:Q\"], ms = 60 },\n]\n",
        );
        let y = pressed(Buttons::Y);
        let start = m.map(&y, 0);
        // Y no llega al juego: lanza la macro.
        assert_eq!(start.pad.buttons, Buttons::A);
        // Sigue aunque se suelte Y.
        assert_eq!(m.map(&pressed(0), 59).pad.buttons, Buttons::A);
        assert_eq!(m.map(&pressed(0), 60).pad.buttons, 0);
        let last = m.map(&pressed(0), 100);
        assert_eq!(last.pad.buttons, Buttons::B);
        assert_eq!(last.keys, [keys::parse_key("Q").unwrap()]);
        // Terminada.
        assert_eq!(m.map(&pressed(0), 160), Mapped::default());
    }

    #[test]
    fn macro_does_not_restart_while_running() {
        let mut m = mapper(
            "[botones]\nY = \"macro:m\"\n[macros.m]\npasos = [{ pulsar = [\"A\"], ms = 100 }]\n",
        );
        m.map(&pressed(Buttons::Y), 0);
        m.map(&pressed(0), 10);
        // Segunda pulsación con la macro aún en marcha: se ignora.
        m.map(&pressed(Buttons::Y), 50);
        assert_eq!(m.map(&pressed(0), 100).pad.buttons, 0);
    }

    #[test]
    fn macro_cannot_launch_macro() {
        let profile = Profile::parse(
            "[macros.a]\npasos = [{ pulsar = [\"macro:a\"], ms = 10 }]\n",
        )
        .unwrap();
        assert!(Mapper::new(&profile).unwrap_err().contains("no puede lanzar otra"));
    }

    #[test]
    fn reset_stops_running_macros() {
        let mut m = mapper(
            "[botones]\nY = \"macro:m\"\n[macros.m]\npasos = [{ pulsar = [\"A\"], ms = 100 }]\n",
        );
        m.map(&pressed(Buttons::Y), 0);
        m.reset();
        assert_eq!(m.map(&pressed(0), 10).pad.buttons, 0);
    }
}
