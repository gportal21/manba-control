//! Perfil de configuración, guardado como TOML en
//! `%APPDATA%\manba-control\perfil.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Contenido del perfil que se crea la primera vez. Explica cada opción
/// para poder editarlo a mano mientras no exista la interfaz gráfica.
pub const TEMPLATE: &str = r#"# Perfil de Manba Control
# Guarda los cambios y usa "Recargar perfil" en el icono de la bandeja.

# Cada cuántos milisegundos se lee el mando (1-16). 1 = mínima latencia.
intervalo_ms = 1

[stick_izquierdo]
# "stick" (normal), "raton" (mueve el cursor) o "ninguno".
modo = "stick"
# Radio de la zona muerta, de 0 a 1.
zona_muerta = 0.25
# A partir de este radio el stick cuenta como llevado al tope.
zona_exterior = 1.0
# 1 = respuesta lineal; mayor que 1 = más precisión cerca del centro.
curva = 1.0
invertir_y = false
# Píxeles por segundo con el stick al tope (solo en modo "raton").
velocidad_raton = 1500.0

[stick_derecho]
modo = "stick"
zona_muerta = 0.25
zona_exterior = 1.0
curva = 1.0
invertir_y = false
velocidad_raton = 1500.0

[gatillos]
# Por debajo de este valor (0-255) el gatillo se ignora.
umbral = 30

[botones]
# Reasignaciones, en la forma  control = "acción".
# Lo que no aparezca aquí funciona igual que siempre.
#
# Controles: A B X Y LB RB LT RT Back Start L3 R3 Arriba Abajo Izquierda Derecha
#
# Acciones:
#   otro control del mando     A = "B"
#   una tecla                  LB = "tecla:Q"
#   un botón del ratón         RB = "raton:izquierdo"   (izquierdo, derecho, medio)
#   desactivar el control      Back = "ninguno"
#
# Teclas: A-Z, 0-9, F1-F12, Esc, Tab, Intro, Espacio, Shift, Ctrl, Alt,
# Retroceso, BloqMayus, Insert, Supr, Inicio, Fin, RePag, AvPag,
# FlechaArriba, FlechaAbajo, FlechaIzquierda, FlechaDerecha
"#;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    #[serde(rename = "intervalo_ms")]
    pub poll_interval_ms: u64,
    #[serde(rename = "stick_izquierdo")]
    pub left_stick: StickConfig,
    #[serde(rename = "stick_derecho")]
    pub right_stick: StickConfig,
    #[serde(rename = "gatillos")]
    pub triggers: TriggerConfig,
    /// Control de origen -> acción, tal como se escribió en el archivo.
    /// Se valida al construir el `Mapper`.
    #[serde(rename = "botones")]
    pub buttons: BTreeMap<String, String>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            poll_interval_ms: 1,
            left_stick: StickConfig::default(),
            right_stick: StickConfig::default(),
            triggers: TriggerConfig::default(),
            buttons: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StickMode {
    #[serde(rename = "stick")]
    Stick,
    #[serde(rename = "raton")]
    Mouse,
    #[serde(rename = "ninguno")]
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StickConfig {
    #[serde(rename = "modo")]
    pub mode: StickMode,
    #[serde(rename = "zona_muerta")]
    pub deadzone: f32,
    #[serde(rename = "zona_exterior")]
    pub outer: f32,
    #[serde(rename = "curva")]
    pub curve: f32,
    #[serde(rename = "invertir_y")]
    pub invert_y: bool,
    #[serde(rename = "velocidad_raton")]
    pub mouse_speed: f32,
}

impl Default for StickConfig {
    fn default() -> Self {
        Self {
            mode: StickMode::Stick,
            deadzone: 0.25,
            outer: 1.0,
            curve: 1.0,
            invert_y: false,
            mouse_speed: 1500.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TriggerConfig {
    #[serde(rename = "umbral")]
    pub threshold: u8,
}

impl Default for TriggerConfig {
    fn default() -> Self {
        Self { threshold: 30 }
    }
}

impl Profile {
    pub fn parse(text: &str) -> Result<Self, String> {
        let profile: Self = toml::from_str(text).map_err(|e| e.message().to_string())?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("no se pudo leer {}: {e}", path.display()))?;
        Self::parse(&text)
    }

    fn validate(&self) -> Result<(), String> {
        if !(1..=16).contains(&self.poll_interval_ms) {
            return Err("intervalo_ms debe estar entre 1 y 16".into());
        }
        for (name, stick) in [
            ("stick_izquierdo", &self.left_stick),
            ("stick_derecho", &self.right_stick),
        ] {
            if !(stick.outer > 0.0 && stick.outer <= 1.0) {
                return Err(format!("{name}: zona_exterior debe estar entre 0 y 1"));
            }
            if !(stick.deadzone >= 0.0 && stick.deadzone < stick.outer) {
                return Err(format!(
                    "{name}: zona_muerta debe estar entre 0 y zona_exterior"
                ));
            }
            if !(stick.curve > 0.0 && stick.curve <= 5.0) {
                return Err(format!("{name}: curva debe estar entre 0 y 5"));
            }
            if !(stick.mouse_speed >= 0.0 && stick.mouse_speed.is_finite()) {
                return Err(format!("{name}: velocidad_raton no puede ser negativa"));
            }
        }
        Ok(())
    }
}

/// `%APPDATA%\manba-control\perfil.toml`.
pub fn default_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("manba-control").join("perfil.toml"))
}

/// Crea el perfil con la plantilla si todavía no existe.
pub fn ensure_exists(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, TEMPLATE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_matches_defaults() {
        assert_eq!(Profile::parse(TEMPLATE).unwrap(), Profile::default());
    }

    #[test]
    fn partial_profile_uses_defaults() {
        let profile = Profile::parse("[stick_derecho]\nmodo = \"raton\"\n").unwrap();
        assert_eq!(profile.right_stick.mode, StickMode::Mouse);
        assert_eq!(profile.right_stick.deadzone, 0.25);
        assert_eq!(profile.left_stick, StickConfig::default());
    }

    #[test]
    fn reads_button_bindings() {
        let profile = Profile::parse("[botones]\nA = \"B\"\nLB = \"tecla:Q\"\n").unwrap();
        assert_eq!(profile.buttons["A"], "B");
        assert_eq!(profile.buttons["LB"], "tecla:Q");
    }

    #[test]
    fn rejects_unknown_option() {
        assert!(Profile::parse("velocidad = 3\n").is_err());
    }

    #[test]
    fn rejects_out_of_range_values() {
        assert!(Profile::parse("intervalo_ms = 0\n").is_err());
        assert!(Profile::parse("[stick_izquierdo]\nzona_muerta = 1.0\n").is_err());
        assert!(Profile::parse("[stick_izquierdo]\ncurva = 0\n").is_err());
    }
}
