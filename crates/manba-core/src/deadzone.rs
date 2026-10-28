//! Zonas muertas de sticks y gatillos.

/// Zonas muertas por defecto de XInput, normalizadas a 0..1.
pub const DEFAULT_LEFT_STICK: f32 = 7849.0 / 32767.0;
pub const DEFAULT_RIGHT_STICK: f32 = 8689.0 / 32767.0;
pub const DEFAULT_TRIGGER: u8 = 30;

/// Convierte un eje crudo (-32768..32767) a -1..1.
pub fn normalize_axis(value: i16) -> f32 {
    (value as f32 / 32767.0).max(-1.0)
}

/// Zona muerta radial con reescalado.
///
/// Por debajo de `inner` el stick queda en reposo; entre `inner` y `outer`
/// el recorrido se reescala a 0..1, así no hay un salto al salir de la zona
/// muerta; a partir de `outer` el stick se considera al máximo.
/// Devuelve los ejes normalizados a -1..1.
pub fn radial(x: i16, y: i16, inner: f32, outer: f32) -> (f32, f32) {
    let (x, y) = (normalize_axis(x), normalize_axis(y));
    let magnitude = x.hypot(y);
    if magnitude <= inner {
        return (0.0, 0.0);
    }
    let scaled = ((magnitude - inner) / (outer - inner)).min(1.0);
    (x / magnitude * scaled, y / magnitude * scaled)
}

/// Curva de respuesta sobre un stick ya normalizado.
///
/// Eleva la distancia al centro a `exponent` manteniendo la dirección:
/// 1 es lineal; mayor que 1 da más precisión en recorridos cortos.
pub fn curve(x: f32, y: f32, exponent: f32) -> (f32, f32) {
    let magnitude = x.hypot(y);
    if magnitude == 0.0 {
        return (0.0, 0.0);
    }
    let factor = magnitude.powf(exponent) / magnitude;
    (x * factor, y * factor)
}

/// Gatillo con umbral, reescalado a 0..1.
pub fn trigger(value: u8, threshold: u8) -> f32 {
    if value <= threshold {
        return 0.0;
    }
    (value - threshold) as f32 / (u8::MAX - threshold) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    #[test]
    fn stick_inside_deadzone_is_zero() {
        assert_eq!(radial(5000, -5000, DEFAULT_LEFT_STICK, 1.0), (0.0, 0.0));
    }

    #[test]
    fn stick_full_deflection_is_one() {
        let (x, y) = radial(32767, 0, DEFAULT_LEFT_STICK, 1.0);
        assert!((x - 1.0).abs() < EPS);
        assert!(y.abs() < EPS);
    }

    #[test]
    fn stick_keeps_direction() {
        let (x, y) = radial(-20000, 20000, DEFAULT_LEFT_STICK, 1.0);
        assert!(x < 0.0 && y > 0.0);
        assert!((x + y).abs() < EPS);
    }

    #[test]
    fn stick_past_outer_is_clamped() {
        let (x, y) = radial(30000, 30000, 0.1, 0.9);
        assert!((x.hypot(y) - 1.0).abs() < EPS);
    }

    #[test]
    fn linear_curve_is_identity() {
        assert_eq!(curve(0.3, -0.4, 1.0), (0.3, -0.4));
    }

    #[test]
    fn steeper_curve_reduces_small_inputs_only() {
        let (x, _) = curve(0.5, 0.0, 2.0);
        assert!((x - 0.25).abs() < EPS);
        let (x, _) = curve(1.0, 0.0, 2.0);
        assert!((x - 1.0).abs() < EPS);
    }

    #[test]
    fn trigger_threshold_and_scale() {
        assert_eq!(trigger(DEFAULT_TRIGGER, DEFAULT_TRIGGER), 0.0);
        assert_eq!(trigger(255, DEFAULT_TRIGGER), 1.0);
        assert!(trigger(150, DEFAULT_TRIGGER) > 0.0);
    }
}
