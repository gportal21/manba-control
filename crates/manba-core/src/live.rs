//! Estado en vivo compartido entre el servicio y la ventana de configuración.
//!
//! Se usa memoria compartida de Windows: el servicio escribe una instantánea
//! y la ventana la lee cuando quiere, sin mensajes ni esperas. Un contador de
//! secuencia evita leer una escritura a medias.

use std::ptr::{self, null};
use std::sync::atomic::{fence, AtomicU32, Ordering};

use serde::Serialize;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_ALL_ACCESS,
    FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;

const NAME: &str = "Local\\ManbaControlLive";

/// Sin latido durante este tiempo se considera que el servicio no está en marcha.
const STALE_MS: u64 = 1500;

pub const PAD_IDLE: u8 = 0;
pub const PAD_ACTIVE: u8 = 1;
pub const PAD_NO_DRIVER: u8 = 2;
pub const PAD_ERROR: u8 = 3;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    /// Momento de la última escritura, en milisegundos desde el arranque de Windows.
    pub heartbeat: u64,
    pub connected: bool,
    /// Perfil activo o en modo de paso directo.
    pub enabled: bool,
    pub profile_ok: bool,
    /// Uno de `PAD_*`.
    pub virtual_pad: u8,
    pub raw_buttons: u16,
    pub raw_lt: u8,
    pub raw_rt: u8,
    pub raw_left: [i16; 2],
    pub raw_right: [i16; 2],
    pub out_buttons: u16,
    pub out_lt: u8,
    pub out_rt: u8,
    pub out_left: [i16; 2],
    pub out_right: [i16; 2],
}

#[repr(C)]
struct Block {
    seq: AtomicU32,
    snapshot: Snapshot,
}

pub fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

struct Mapping {
    handle: HANDLE,
    view: *mut Block,
}

// La vista solo se usa a través de `LiveWriter`/`LiveReader`, que no se comparten entre hilos.
unsafe impl Send for Mapping {}

impl Mapping {
    fn map(handle: HANDLE, access: u32) -> Option<Self> {
        if handle.is_null() {
            return None;
        }
        let view = unsafe { MapViewOfFile(handle, access, 0, 0, size_of::<Block>()) };
        if view.Value.is_null() {
            unsafe { CloseHandle(handle) };
            return None;
        }
        Some(Self {
            handle,
            view: view.Value.cast(),
        })
    }

    fn block(&self) -> &Block {
        unsafe { &*self.view }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.view.cast(),
            });
            CloseHandle(self.handle);
        }
    }
}

/// Lado del servicio.
pub struct LiveWriter(Mapping);

impl LiveWriter {
    pub fn create() -> Option<Self> {
        Self::create_named(NAME)
    }

    fn create_named(name: &str) -> Option<Self> {
        let name = wide(name);
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                null(),
                PAGE_READWRITE,
                0,
                size_of::<Block>() as u32,
                name.as_ptr(),
            )
        };
        Mapping::map(handle, FILE_MAP_ALL_ACCESS).map(Self)
    }

    pub fn write(&mut self, snapshot: &Snapshot) {
        let view = self.0.view;
        let seq_counter = unsafe { &(*view).seq };
        // Secuencia impar mientras se escribe.
        let seq = seq_counter.load(Ordering::Relaxed);
        seq_counter.store(seq.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        unsafe { ptr::write_volatile(ptr::addr_of_mut!((*view).snapshot), *snapshot) };
        seq_counter.store(seq.wrapping_add(2), Ordering::Release);
    }
}

/// Lado de la ventana de configuración.
pub struct LiveReader(Mapping);

impl LiveReader {
    /// Falla si el servicio no ha creado todavía la memoria compartida.
    pub fn open() -> Option<Self> {
        Self::open_named(NAME)
    }

    fn open_named(name: &str) -> Option<Self> {
        let name = wide(name);
        let handle = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) };
        Mapping::map(handle, FILE_MAP_READ).map(Self)
    }

    /// Última instantánea, o `None` si el servicio no da señales de vida.
    pub fn read(&self) -> Option<Snapshot> {
        let block = self.0.block();
        for _ in 0..16 {
            let before = block.seq.load(Ordering::Acquire);
            if before % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let snapshot = unsafe { ptr::read_volatile(ptr::addr_of!((*self.0.view).snapshot)) };
            fence(Ordering::Acquire);
            if block.seq.load(Ordering::Relaxed) == before {
                let alive = now_ms().saturating_sub(snapshot.heartbeat) < STALE_MS;
                return alive.then_some(snapshot);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_and_reader_share_snapshot() {
        let name = "Local\\ManbaControlTest1";
        let mut writer = LiveWriter::create_named(name).unwrap();
        let reader = LiveReader::open_named(name).unwrap();
        let snapshot = Snapshot {
            heartbeat: now_ms(),
            connected: true,
            raw_left: [100, -200],
            out_buttons: 0x1000,
            ..Default::default()
        };
        writer.write(&snapshot);
        assert_eq!(reader.read(), Some(snapshot));
    }

    #[test]
    fn stale_snapshot_reads_as_offline() {
        let name = "Local\\ManbaControlTest2";
        let mut writer = LiveWriter::create_named(name).unwrap();
        let reader = LiveReader::open_named(name).unwrap();
        writer.write(&Snapshot::default());
        assert_eq!(reader.read(), None);
    }
}
