//! Registro de eventos en `%APPDATA%\manba-control\servicio.log`, para poder
//! saber qué pasó cuando algo falla. Solo se escribe en eventos (conexiones,
//! errores, cambios de perfil), nunca en cada lectura del mando.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows_sys::Win32::System::SystemInformation::GetLocalTime;

/// Al pasar de este tamaño se empieza un archivo nuevo.
const MAX_BYTES: u64 = 256 * 1024;

static PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init(path: &Path) {
    let _ = PATH.set(path.to_path_buf());
}

pub fn write(message: &str) {
    let Some(path) = PATH.get() else {
        return;
    };
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
    let mut time = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut time) };
    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}  {message}\n",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond, time.wMilliseconds
    );
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}
