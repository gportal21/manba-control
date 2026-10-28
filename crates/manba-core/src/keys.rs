//! Teclas que se pueden asignar a los botones del mando.
//!
//! Se envían por código de exploración (scancode) y no por tecla virtual,
//! porque muchos juegos leen el teclado a bajo nivel e ignoran las teclas
//! virtuales. Son posiciones físicas: "Q" es la tecla de esa posición en
//! cualquier distribución de teclado.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub scancode: u16,
    /// Teclas del bloque extendido (flechas, Inicio, Supr...).
    pub extended: bool,
}

const fn key(scancode: u16) -> Key {
    Key { scancode, extended: false }
}

const fn ext(scancode: u16) -> Key {
    Key { scancode, extended: true }
}

const KEYS: &[(&str, Key)] = &[
    ("Esc", key(0x01)),
    ("1", key(0x02)),
    ("2", key(0x03)),
    ("3", key(0x04)),
    ("4", key(0x05)),
    ("5", key(0x06)),
    ("6", key(0x07)),
    ("7", key(0x08)),
    ("8", key(0x09)),
    ("9", key(0x0A)),
    ("0", key(0x0B)),
    ("Retroceso", key(0x0E)),
    ("Tab", key(0x0F)),
    ("Q", key(0x10)),
    ("W", key(0x11)),
    ("E", key(0x12)),
    ("R", key(0x13)),
    ("T", key(0x14)),
    ("Y", key(0x15)),
    ("U", key(0x16)),
    ("I", key(0x17)),
    ("O", key(0x18)),
    ("P", key(0x19)),
    ("Intro", key(0x1C)),
    ("Ctrl", key(0x1D)),
    ("A", key(0x1E)),
    ("S", key(0x1F)),
    ("D", key(0x20)),
    ("F", key(0x21)),
    ("G", key(0x22)),
    ("H", key(0x23)),
    ("J", key(0x24)),
    ("K", key(0x25)),
    ("L", key(0x26)),
    ("Shift", key(0x2A)),
    ("Z", key(0x2C)),
    ("X", key(0x2D)),
    ("C", key(0x2E)),
    ("V", key(0x2F)),
    ("B", key(0x30)),
    ("N", key(0x31)),
    ("M", key(0x32)),
    ("Alt", key(0x38)),
    ("Espacio", key(0x39)),
    ("BloqMayus", key(0x3A)),
    ("F1", key(0x3B)),
    ("F2", key(0x3C)),
    ("F3", key(0x3D)),
    ("F4", key(0x3E)),
    ("F5", key(0x3F)),
    ("F6", key(0x40)),
    ("F7", key(0x41)),
    ("F8", key(0x42)),
    ("F9", key(0x43)),
    ("F10", key(0x44)),
    ("F11", key(0x57)),
    ("F12", key(0x58)),
    ("Inicio", ext(0x47)),
    ("FlechaArriba", ext(0x48)),
    ("RePag", ext(0x49)),
    ("FlechaIzquierda", ext(0x4B)),
    ("FlechaDerecha", ext(0x4D)),
    ("Fin", ext(0x4F)),
    ("FlechaAbajo", ext(0x50)),
    ("AvPag", ext(0x51)),
    ("Insert", ext(0x52)),
    ("Supr", ext(0x53)),
];

/// Busca una tecla por nombre, sin distinguir mayúsculas.
pub fn parse_key(name: &str) -> Option<Key> {
    let name = name.trim();
    KEYS.iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|&(_, key)| key)
}

/// Nombres válidos, para los mensajes de error.
pub fn key_names() -> impl Iterator<Item = &'static str> {
    KEYS.iter().map(|&(name, _)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ignoring_case_and_spaces() {
        assert_eq!(parse_key(" espacio "), Some(key(0x39)));
        assert_eq!(parse_key("q"), Some(key(0x10)));
    }

    #[test]
    fn arrows_are_extended() {
        assert!(parse_key("FlechaArriba").unwrap().extended);
        assert!(!parse_key("W").unwrap().extended);
    }

    #[test]
    fn unknown_key() {
        assert_eq!(parse_key("Tecla rara"), None);
    }

    #[test]
    fn names_are_unique() {
        let names: Vec<_> = key_names().map(str::to_ascii_lowercase).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len());
    }
}
