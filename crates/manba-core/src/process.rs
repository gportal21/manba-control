//! Qué programas hay abiertos y cuál está en primer plano, por el nombre
//! de su ejecutable. Se usa para activar el perfil de cada juego.

use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowThreadProcessId,
    IsWindowVisible,
};

/// Nombre del ejecutable de un proceso, en minúsculas (`juego.exe`).
fn exe_name(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let mut buffer = [0u16; 1024];
    let mut len = buffer.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut len)
    };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buffer[..len as usize]);
    let name = path.rsplit(['\\', '/']).next()?.to_lowercase();
    (!name.is_empty()).then_some(name)
}

fn window_exe(hwnd: HWND) -> Option<String> {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    exe_name(pid)
}

/// Ejecutable de la ventana en primer plano.
pub fn foreground_exe() -> Option<String> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return None;
    }
    window_exe(hwnd)
}

/// Programas del sistema que nunca son un juego.
const IGNORED: &[&str] = &[
    "applicationframehost.exe",
    "explorer.exe",
    "manba-service.exe",
    "manba-ui.exe",
    "searchhost.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "systemsettings.exe",
    "textinputhost.exe",
];

/// Ejecutables con alguna ventana visible, ordenados y sin repetir.
pub fn visible_apps() -> Vec<String> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let windows = &mut *(lparam as *mut Vec<HWND>);
        if IsWindowVisible(hwnd) != 0 && GetWindowTextLengthW(hwnd) > 0 {
            windows.push(hwnd);
        }
        1
    }

    let mut windows: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(collect), &mut windows as *mut Vec<HWND> as LPARAM) };

    let mut apps: Vec<String> = windows
        .into_iter()
        .filter_map(window_exe)
        .filter(|name| !IGNORED.contains(&name.as_str()))
        .collect();
    apps.sort();
    apps.dedup();
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_has_a_name() {
        let name = exe_name(std::process::id()).unwrap();
        assert!(name.ends_with(".exe"));
        assert_eq!(name, name.to_lowercase());
    }

    #[test]
    fn invalid_pid_has_no_name() {
        assert_eq!(exe_name(0), None);
    }
}
