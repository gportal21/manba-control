//! Hilo de trabajo: lee el mando, elige el perfil, lo aplica y envía el resultado.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use manba_core::live::{self, LiveWriter, Snapshot};
use manba_core::mapping::{Mapper, PadOutput};
use manba_core::store::{AppConfig, Store, DEFAULT_PROFILE};
use manba_core::xinput::{self, Buttons, Event, GamepadState, Poller};
use manba_core::process;

use crate::log;
use crate::output::{KeyboardMouse, VirtualPad, VirtualPadError};

/// Sin mando conectado no hace falta leer tan a menudo.
const IDLE_INTERVAL: Duration = Duration::from_millis(50);
/// Tope del tiempo entre ciclos para el movimiento del cursor, por si el
/// hilo se retrasa (evita saltos bruscos).
const MAX_STEP: Duration = Duration::from_millis(50);
/// Cada cuánto se mira si algún perfil o la configuración han cambiado.
const PROFILE_CHECK: Duration = Duration::from_millis(500);
/// Cada cuánto se mira qué programa está en primer plano.
const FOREGROUND_CHECK: Duration = Duration::from_millis(250);
/// Cada cuánto se publica el estado aunque no cambie, como señal de vida.
const HEARTBEAT: Duration = Duration::from_millis(250);

/// Espera entre intentos de crear el mando virtual. Justo después de cerrar
/// otra instancia, el driver puede tardar un momento en liberar el anterior.
const PAD_RETRY: Duration = Duration::from_secs(1);

/// Botones que usan los atajos mientras se mantiene Back.
const SHORTCUT_BUTTONS: u16 = Buttons::DPAD_LEFT | Buttons::DPAD_RIGHT | Buttons::DPAD_DOWN;

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
    pub enabled: bool,
    pub profiles: Vec<String>,
    pub base: String,
    pub shortcuts: bool,
    pub active: String,
    /// Uno de `live::REASON_*`.
    pub reason: u8,
    /// Juego que activó el perfil, si fue por juego.
    pub game: Option<String>,
    /// Perfiles que no se pudieron cargar, con su error.
    pub errors: Vec<String>,
}

/// Estado compartido entre la bandeja (hilo principal) y el motor.
pub struct Shared {
    /// Con la reasignación en pausa el mando virtual repite al físico.
    pub enabled: AtomicBool,
    pub stop: AtomicBool,
    pub status: Mutex<Status>,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            status: Mutex::new(Status::default()),
        }
    }
}

struct Loaded {
    name: String,
    games: Vec<String>,
    mapper: Mapper,
    interval: Duration,
    vibration: f32,
}

impl Loaded {
    fn fallback() -> Self {
        Self {
            name: DEFAULT_PROFILE.to_string(),
            games: Vec::new(),
            mapper: Mapper::default(),
            interval: Duration::from_millis(1),
            vibration: 1.0,
        }
    }
}

fn load_profiles(store: &Store) -> (Vec<Loaded>, Vec<String>) {
    let mut loaded = Vec::new();
    let mut errors = Vec::new();
    for name in store.list() {
        let result = store
            .load(&name)
            .and_then(|profile| Mapper::new(&profile).map(|mapper| (profile, mapper)));
        match result {
            Ok((profile, mapper)) => loaded.push(Loaded {
                games: profile.game_keys(),
                interval: Duration::from_millis(profile.poll_interval_ms),
                vibration: profile.vibration.intensity,
                mapper,
                name,
            }),
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    if loaded.is_empty() {
        loaded.push(Loaded::fallback());
    }
    (loaded, errors)
}

/// Perfil elegido con el mando. Dura mientras siga en primer plano el
/// mismo programa en el que se eligió.
struct Override {
    profile: String,
    exe: Option<String>,
}

/// Intentos de crear el mando virtual, para reintentar sin llenar el registro.
#[derive(Default)]
struct PadAttempts {
    failures: u32,
    next_retry: Option<Instant>,
}

impl PadAttempts {
    fn due(&self) -> bool {
        self.next_retry.is_some_and(|at| Instant::now() >= at)
    }

    /// Intenta crear el mando virtual para el mando físico de `slot`.
    fn connect(
        &mut self,
        slot: u32,
        vibration: &Arc<AtomicU32>,
        status: &mut Status,
    ) -> Option<VirtualPad> {
        match VirtualPad::connect(slot, Arc::clone(vibration)) {
            Ok(virtual_pad) => {
                match self.failures {
                    0 => log::write("Mando virtual creado"),
                    1 => log::write("Mando virtual creado tras 1 intento fallido"),
                    n => log::write(&format!("Mando virtual creado tras {n} intentos fallidos")),
                }
                *self = Self::default();
                status.virtual_pad = PadStatus::Active;
                Some(virtual_pad)
            }
            Err(VirtualPadError::NoDriver) => {
                log::write("No se pudo crear el mando virtual: ViGEmBus no está instalado");
                *self = Self::default();
                status.virtual_pad = PadStatus::NoDriver;
                None
            }
            Err(VirtualPadError::Other(e)) => {
                self.failures += 1;
                // El primero y luego uno de cada 60 (un minuto), para no llenar el registro.
                if self.failures % 60 == 1 {
                    log::write(&format!(
                        "No se pudo crear el mando virtual (intento {}): {e}. Se reintentará",
                        self.failures
                    ));
                }
                self.next_retry = Some(Instant::now() + PAD_RETRY);
                status.virtual_pad = PadStatus::Error(e);
                None
            }
        }
    }

    /// El mando virtual dejó de responder: se reintenta en breve.
    fn lost(&mut self) {
        self.failures = 0;
        self.next_retry = Some(Instant::now() + PAD_RETRY);
    }
}

/// Unas vibraciones cortas en el mando físico para confirmar un atajo.
fn pulse(slot: u32, count: u32) {
    thread::spawn(move || {
        for _ in 0..count {
            let _ = xinput::set_vibration(slot, 45_000, 45_000);
            thread::sleep(Duration::from_millis(90));
            let _ = xinput::set_vibration(slot, 0, 0);
            thread::sleep(Duration::from_millis(90));
        }
    });
}

/// Bucle principal del motor. `notify` avisa a la bandeja de que el estado cambió.
pub fn run(shared: &Shared, store: &Store, notify: impl Fn()) {
    let mut passthrough = Mapper::default();
    let vibration = Arc::new(AtomicU32::new(1.0f32.to_bits()));
    // Reloj del turbo y las macros.
    let clock = Instant::now();
    let mut last_active: Option<String> = None;

    let mut profiles: Vec<Loaded> = vec![Loaded::fallback()];
    let mut config = AppConfig::default();
    let mut signature: Vec<(String, Option<SystemTime>)> = Vec::new();
    let mut last_profile_check: Option<Instant> = None;
    let mut foreground: Option<String> = None;
    let mut last_foreground_check: Option<Instant> = None;
    let mut override_: Option<Override> = None;
    let mut active_index = 0;
    let mut prev_buttons = 0u16;

    let mut poller = Poller::new();
    let mut events = Vec::new();
    let mut active: Option<(u32, GamepadState)> = None;
    let mut pad: Option<VirtualPad> = None;
    let mut pad_attempts = PadAttempts::default();
    let mut last_pad: Option<PadOutput> = None;
    let mut kbm = KeyboardMouse::default();

    let mut status = Status::default();
    let mut published: Option<Status> = None;
    let mut last_tick = Instant::now();

    let mut live_writer = LiveWriter::create();
    let mut last_snapshot = Snapshot::default();
    let mut last_live_write: Option<Instant> = None;

    let due = |last: &Option<Instant>, every: Duration| last.map_or(true, |at| at.elapsed() >= every);

    while !shared.stop.load(Ordering::Relaxed) {
        // Perfiles y configuración: se recargan solos al cambiar cualquier archivo.
        if due(&last_profile_check, PROFILE_CHECK) {
            last_profile_check = Some(Instant::now());
            let current = store.signature();
            if current != signature {
                signature = current;
                let (loaded, errors) = load_profiles(store);
                for error in &errors {
                    log::write(&format!("Error en el perfil {error}"));
                }
                profiles = loaded;
                status.errors = errors;
                config = store.load_config();
            }
        }

        if due(&last_foreground_check, FOREGROUND_CHECK) {
            last_foreground_check = Some(Instant::now());
            foreground = process::foreground_exe();
        }

        events.clear();
        poller.poll(&mut events);
        for event in &events {
            match *event {
                Event::Connected(slot) if active.is_none() => {
                    log::write(&format!("Mando conectado en la ranura {}", slot + 1));
                    active = Some((slot, GamepadState::default()));
                    pad = pad_attempts.connect(slot, &vibration, &mut status);
                }
                // Con el mando virtual creado, lo que aparezca nuevo es él
                // mismo. ViGEm no informa de forma fiable en qué ranura de
                // XInput queda, así que se detecta aquí y se deja de leer.
                Event::Connected(slot) if pad.is_some() => {
                    log::write(&format!("Mando virtual detectado en la ranura {}", slot + 1));
                    poller.set_ignored(Some(slot));
                }
                Event::Connected(slot) => {
                    log::write(&format!("Otro mando en la ranura {} (se ignora)", slot + 1));
                }
                Event::Disconnected(slot) if active.is_some_and(|(s, _)| s == slot) => {
                    log::write(&format!("Mando desconectado de la ranura {}", slot + 1));
                    active = None;
                    pad = None;
                    pad_attempts = PadAttempts::default();
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

        if let Some((slot, _)) = active {
            if pad.is_none() && pad_attempts.due() {
                pad = pad_attempts.connect(slot, &vibration, &mut status);
            }
        }

        // Atajos: con Back pulsado, la cruceta cambia de perfil o pausa. Esos
        // botones de la cruceta no llegan al juego mientras se mantenga Back.
        let mut input = active.map(|(_, state)| state);
        if let (Some((slot, state)), Some(input)) = (&active, &mut input) {
            let buttons = state.buttons.0;
            if config.shortcuts && state.buttons.is_pressed(Buttons::BACK) {
                let newly = buttons & !prev_buttons;
                let step = if newly & Buttons::DPAD_RIGHT != 0 {
                    Some(1)
                } else if newly & Buttons::DPAD_LEFT != 0 {
                    Some(profiles.len() - 1)
                } else {
                    None
                };
                if let Some(step) = step {
                    let next = (active_index + step) % profiles.len();
                    override_ = Some(Override {
                        profile: profiles[next].name.clone(),
                        exe: foreground.clone(),
                    });
                    pulse(*slot, 1);
                }
                if newly & Buttons::DPAD_DOWN != 0 {
                    let enabled = !shared.enabled.load(Ordering::Relaxed);
                    shared.enabled.store(enabled, Ordering::Relaxed);
                    pulse(*slot, if enabled { 1 } else { 2 });
                }
                input.buttons.0 &= !SHORTCUT_BUTTONS;
            }
            prev_buttons = buttons;
        }

        // Qué perfil aplicar: el elegido con el mando, el del juego en primer
        // plano o el perfil base, en ese orden.
        if override_
            .as_ref()
            .is_some_and(|o| o.exe != foreground || !profiles.iter().any(|p| p.name == o.profile))
        {
            override_ = None;
        }
        let game_index = foreground
            .as_ref()
            .and_then(|exe| profiles.iter().position(|p| p.games.contains(exe)));
        let (index, reason) = if let Some(o) = &override_ {
            let index = profiles.iter().position(|p| p.name == o.profile).unwrap_or(0);
            (index, live::REASON_SHORTCUT)
        } else if let Some(index) = game_index {
            (index, live::REASON_GAME)
        } else {
            let index = profiles
                .iter()
                .position(|p| p.name == config.base_profile)
                .unwrap_or(0);
            (index, live::REASON_BASE)
        };
        active_index = index;
        let profile = &mut profiles[index];
        if last_active.as_deref() != Some(profile.name.as_str()) {
            let why = match reason {
                live::REASON_GAME => format!("juego {}", foreground.as_deref().unwrap_or("?")),
                live::REASON_SHORTCUT => "elegido con el mando".to_string(),
                _ => "perfil base".to_string(),
            };
            log::write(&format!("Perfil activo: {} ({why})", profile.name));
            // Un perfil recién activado empieza sin macros a medias.
            profile.mapper.reset();
            last_active = Some(profile.name.clone());
        }
        vibration.store(profile.vibration.to_bits(), Ordering::Relaxed);

        let now = Instant::now();
        let elapsed = (now - last_tick).min(MAX_STEP);
        last_tick = now;

        let enabled = shared.enabled.load(Ordering::Relaxed);
        let mut snapshot = Snapshot {
            enabled,
            profile_ok: status.errors.is_empty(),
            reason,
            ..Default::default()
        };
        snapshot.set_active_profile(&profile.name);

        if let (Some((_, state)), Some(input)) = (&active, &input) {
            let mapper = if enabled { &mut profile.mapper } else { &mut passthrough };
            let mapped = mapper.map(input, clock.elapsed().as_millis() as u64);

            if let Some(virtual_pad) = &mut pad {
                if last_pad != Some(mapped.pad) {
                    match virtual_pad.update(&mapped.pad) {
                        Ok(()) => last_pad = Some(mapped.pad),
                        Err(e) => {
                            log::write(&format!(
                                "El mando virtual dejó de responder: {e}. Se volverá a crear"
                            ));
                            pad = None;
                            last_pad = None;
                            poller.set_ignored(None);
                            pad_attempts.lost();
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
            if snapshot != last_snapshot || due(&last_live_write, HEARTBEAT) {
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
        status.enabled = enabled;
        status.profiles = profiles.iter().map(|p| p.name.clone()).collect();
        status.base = config.base_profile.clone();
        status.shortcuts = config.shortcuts;
        status.active = profiles[index].name.clone();
        status.reason = reason;
        status.game = (reason == live::REASON_GAME).then(|| foreground.clone()).flatten();
        if published.as_ref() != Some(&status) {
            published = Some(status.clone());
            *shared.status.lock().unwrap() = status.clone();
            notify();
        }

        let interval = if active.is_some() { profiles[index].interval } else { IDLE_INTERVAL };
        thread::sleep(interval);
    }
}
