//! Núcleo de Manba Control: lectura del mando y procesamiento de su entrada.
//!
//! En modo PC el Manba One se presenta como un mando de Xbox 360
//! (VID 045E / PID 028E), así que se lee mediante XInput.

pub mod autostart;
pub mod deadzone;
pub mod keys;
pub mod live;
pub mod mapping;
pub mod profile;
pub mod xinput;
