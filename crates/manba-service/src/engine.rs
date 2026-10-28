//! Hilo de trabajo: lee el mando, aplica el perfil y envía el resultado.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use manba_core::live::{self, LiveWriter, Snapshot};
use manba_core::mapping::{Mapper, PadOutput};
use manba_core::profile::Profile;
use manba_core::xinput::{Event, GamepadState, Poller};

use crate::output::{KeyboardMouse, VirtualPad, VirtualPadError};

/// Sin mando conectado no hace falta leer tan a menudo.
const IDLE_INTERVAL: Duration = Duration::from_millis(50);
/// Tope del tiempo entre ciclos para el movimiento del cursor, por si el
/// hilo se retrasa (evita saltos bruscos).
const MAX_STEP: Duration = Duration::from_millis(50);
/// Cada cuánto se mira si el archivo del perfil ha cambiado.
const PROFILE_CHECK: Duration = Duration::from_millis(500);
/// Cada cuánto se publica el estado aunque no cambie, como señal de vida.
const HEARTBEAT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PadStatus {
    #[default]
    Idle,
    Active,
    NoDriver,
    Error(String),
}

impl PadStatus {
    fn code(&self) -> u8 {
        match self {
            Self::Idle => live::PAD_IDLE,
            Self::Active => live::PAD_ACTIVE,
            Self::NoDriver => live::PAD_NO_DRIVER,
            Self::Error(_) => live::PAD_ERROR,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// Ranura de XInput del mando físico en uso.
    pub controller: Option<u32>,
    pub virtual_pad: PadStatus,
    pub profile_error: Option<String>,
}

/// Estado compartido entre la bandeja (hilo principal) y el motor.
pub struct Shared {
    /// Con el perfil desactivado el mando virtual repite al físico.
    pub enabled: AtomicBool,
    pub reload: AtomicBool,
    pub stop: AtomicBool,
    pub status: Mutex<Status>,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            reload: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            status: Mutex::new(Status::default()),
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Bucle principal del motor. `notify` avisa a la bandeja de que el estado cambió.
pub fn run(shared: &Shared, profile_path: &Path, notify: impl Fn()) {
    let passthrough = Mapper::default();
    let mut mapper = Mapper::default();
    let mut interval = Duration::from_millis(1);
    let vibration = Arc::new(AtomicU32::new(1.0f32.to_bits()));
    let mut profile_mtime = None;
    let mut last_profile_check = Instant::now();

    let mut poller = Poller::new();
    let mut events = Vec::new();
    let mut active: Option<(u32, GamepadState)> = None;
    let mut pad: Option<VirtualPad> = None;
    let mut last_pad: Option<PadOutput> = None;
    let mut kbm = KeyboardMouse::default();

    let mut status = Status::default();
    let mut published = Status::default();
    let mut last_tick = Instant::now();

    let mut live_writer = LiveWriter::create();
    let mut last_snapshot = Snapshot::default();
    let mut last_live_write: Option<Instant> = None;

    while !shared.stop.load(Ordering::Relaxed) {
        // Recarga automática cuando cambia el archivo (por ejemplo, al
        // guardar desde la ventana de configuración).
        if last_profile_check.elapsed() >= PROFILE_CHECK {
            last_profile_check = Instant::now();
            let mtime = modified(profile_path);
            if mtime != profile_mtime {
                shared.reload.store(true, Ordering::Relaxed);
            }
        }
        if shared.reload.swap(false, Ordering::Relaxed) {
            profile_mtime = modified(profile_path);
            let loaded = Profile::load(profile_path)
                .and_then(|profile| Mapper::new(&profile).map(|m| (profile, m)));
            match loaded {
                Ok((profile, new_mapper)) => {
                    interval = Duration::from_millis(profile.poll_interval_ms);
                    vibration.store(profile.vibration.intensity.to_bits(), Ordering::Relaxed);
                    mapper = new_mapper;
                    status.profile_error = None;
                }
                // Se mantiene el perfil anterior hasta que se corrija el archivo.
                Err(e) => status.profile_error = Some(e),
            }
        }

        events.clear();
        poller.poll(&mut events);
        for event in &events {
            match *event {
                Event::Connected(slot) if active.is_none() => {
                    active = Some((slot, GamepadState::default()));
                    match VirtualPad::connect(slot, Arc::clone(&vibration)) {
                        Ok(virtual_pad) => {
                            pad = Some(virtual_pad);
                            status.virtual_pad = PadStatus::Active;
                        }
                        Err(VirtualPadError::NoDriver) => status.virtual_pad = PadStatus::NoDriver,
                        Err(VirtualPadError::Other(e)) => {
                            status.virtual_pad = PadStatus::Error(e)
                        }
                    }
                }
                // Con el mando virtual creado, lo que aparezca nuevo es él
                // mismo. ViGEm no informa de forma fiable en qué ranura de
                // XInput queda, así que se detecta aquí y se deja de leer.
                Event::Connected(slot) if pad.is_some() => poller.set_ignored(Some(slot)),
                Event::Disconnected(slot) if active.is_some_and(|(s, _)| s == slot) => {
                    active = None;
                    pad = None;
                    last_pad = None;
                    poller.set_ignored(None);
                    kbm.release_all();
                    status.virtual_pad = PadStatus::Idle;
                }
                Event::State(slot, state) => {
                    if let Some((active_slot, current)) = &mut active {
                        if *active_slot == slot {
                            *current = state;
                        }
                    }
                }
                _ => {}
            }
        }

        let now = Instant::now();
        let elapsed = (now - last_tick).min(MAX_STEP);
        last_tick = now;

        let enabled = shared.enabled.load(Ordering::Relaxed);
        let mut snapshot = Snapshot {
            enabled,
            profile_ok: status.profile_error.is_none(),
            ..Default::default()
        };

        if let Some((_, state)) = &active {
            let current = if enabled { &mapper } else { &passthrough };
            let mapped = current.map(state);

            if let Some(virtual_pad) = &mut pad {
                if last_pad != Some(mapped.pad) {
                    match virtual_pad.update(&mapped.pad) {
                        Ok(()) => last_pad = Some(mapped.pad),
                        Err(e) => {
                            pad = None;
                            last_pad = None;
                            poller.set_ignored(None);
                            status.virtual_pad = PadStatus::Error(e);
                        }
                    }
                }
            }
            kbm.apply(&mapped, elapsed);

            snapshot.connected = true;
            snapshot.raw_buttons = state.buttons.0;
            snapshot.raw_lt = state.left_trigger;
            snapshot.raw_rt = state.right_trigger;
            snapshot.raw_left = [state.left_stick.0, state.left_stick.1];
            snapshot.raw_right = [state.right_stick.0, state.right_stick.1];
            snapshot.out_buttons = mapped.pad.buttons;
            snapshot.out_lt = mapped.pad.left_trigger;
            snapshot.out_rt = mapped.pad.right_trigger;
            snapshot.out_left = [mapped.pad.left_stick.0, mapped.pad.left_stick.1];
            snapshot.out_right = [mapped.pad.right_stick.0, mapped.pad.right_stick.1];
        }
        snapshot.virtual_pad = status.virtual_pad.code();

        if let Some(writer) = &mut live_writer {
            let due = last_live_write.map_or(true, |at| at.elapsed() >= HEARTBEAT);
            if snapshot != last_snapshot || due {
                last_snapshot = snapshot;
                writer.write(&Snapshot {
                    heartbeat: live::now_ms(),
                    ..snapshot
                });
                last_live_write = Some(Instant::now());
            }
        } else {
            live_writer = LiveWriter::create();
        }

        status.controller = active.map(|(slot, _)| slot);
        if status != published {
            published = status.clone();
            *shared.status.lock().unwrap() = status.clone();
            notify();
        }

        thread::sleep(if active.is_some() { interval } else { IDLE_INTERVAL });
    }
}
