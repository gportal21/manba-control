//! Inicio automático con Windows mediante la clave Run del usuario
//! (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`). No necesita
//! permisos de administrador y se ve en el Administrador de tareas, en
//! la pestaña de aplicaciones de inicio.

use std::path::Path;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE: &str = "ManbaControl";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub fn is_enabled() -> bool {
    exists(RUN_KEY, VALUE)
}

/// Registra `exe` para que arranque al iniciar sesión.
pub fn enable(exe: &Path) -> Result<(), String> {
    set(RUN_KEY, VALUE, &format!("\"{}\"", exe.display()))
}

pub fn disable() -> Result<(), String> {
    remove(RUN_KEY, VALUE)
}

fn exists(key: &str, value: &str) -> bool {
    let (key, value) = (wide(key), wide(value));
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    code == ERROR_SUCCESS
}

fn set(key: &str, value: &str, data: &str) -> Result<(), String> {
    let (key, value, data) = (wide(key), wide(value), wide(data));
    let code = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    match code {
        ERROR_SUCCESS => Ok(()),
        other => Err(format!("no se pudo activar el inicio con Windows (error {other})")),
    }
}

fn remove(key: &str, value: &str) -> Result<(), String> {
    let (key, value) = (wide(key), wide(value));
    let code = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
    match code {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        other => Err(format!("no se pudo desactivar el inicio con Windows (error {other})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Usa una clave propia, no la de inicio, para no dejar rastro si el test falla.
    const TEST_KEY: &str = "Software\\ManbaControlTest";

    #[test]
    fn set_check_and_remove() {
        let value = "Prueba";
        let _ = remove(TEST_KEY, value);
        assert!(!exists(TEST_KEY, value));
        set(TEST_KEY, value, "\"C:\\ruta\\manba-service.exe\"").unwrap();
        assert!(exists(TEST_KEY, value));
        remove(TEST_KEY, value).unwrap();
        assert!(!exists(TEST_KEY, value));
        // Borrar algo que no existe no es un error.
        remove(TEST_KEY, value).unwrap();

        let key = wide(TEST_KEY);
        unsafe { windows_sys::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, key.as_ptr()) };
    }
}
