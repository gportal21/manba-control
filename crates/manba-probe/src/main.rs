//! Probador en consola: detecta el mando y muestra en vivo botones,
//! sticks (crudos y con zona muerta) y gatillos.
//!
//! Uso: manba-probe [--vibrar]

use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

use manba_core::deadzone;
use manba_core::xinput::{self, Event, GamepadState, Poller};

/// 250 Hz: de sobra para un probador y con un consumo de CPU despreciable.
const POLL_INTERVAL: Duration = Duration::from_millis(4);
/// La consola no necesita redibujarse más de ~30 veces por segundo.
const DRAW_INTERVAL: Duration = Duration::from_millis(33);

fn main() {
    let vibrate = std::env::args().any(|arg| arg == "--vibrar");

    println!("Manba Probe - esperando mando (Ctrl+C para salir)...");

    let mut poller = Poller::new();
    let mut events = Vec::new();
    let mut pending: Option<(u32, GamepadState)> = None;
    let mut last_draw = Instant::now();

    loop {
        events.clear();
        poller.poll(&mut events);

        for event in &events {
            match *event {
                Event::Connected(slot) => on_connected(slot, vibrate),
                Event::Disconnected(slot) => {
                    clear_line();
                    println!("Mando {} desconectado.", slot + 1);
                    if pending.is_some_and(|(s, _)| s == slot) {
                        pending = None;
                    }
                }
                Event::State(slot, state) => pending = Some((slot, state)),
            }
        }

        if last_draw.elapsed() >= DRAW_INTERVAL {
            if let Some((slot, state)) = pending.take() {
                draw(slot, &state);
                last_draw = Instant::now();
            }
        }

        thread::sleep(POLL_INTERVAL);
    }
}

fn on_connected(slot: u32, vibrate: bool) {
    clear_line();
    let battery = xinput::get_battery(slot)
        .map(|b| b.to_string())
        .unwrap_or_else(|e| e.to_string());
    println!("Mando {} conectado (batería: {battery}).", slot + 1);

    if vibrate {
        // Pulso corto en ambos motores para comprobar la vibración.
        let _ = xinput::set_vibration(slot, 40_000, 40_000);
        thread::sleep(Duration::from_millis(400));
        let _ = xinput::set_vibration(slot, 0, 0);
    }
}

fn draw(slot: u32, state: &GamepadState) {
    let (lx, ly) = deadzone::radial(
        state.left_stick.0,
        state.left_stick.1,
        deadzone::DEFAULT_LEFT_STICK,
        1.0,
    );
    let (rx, ry) = deadzone::radial(
        state.right_stick.0,
        state.right_stick.1,
        deadzone::DEFAULT_RIGHT_STICK,
        1.0,
    );
    let lt = deadzone::trigger(state.left_trigger, deadzone::DEFAULT_TRIGGER);
    let rt = deadzone::trigger(state.right_trigger, deadzone::DEFAULT_TRIGGER);
    let buttons: Vec<_> = state.buttons.pressed_names().collect();

    clear_line();
    print!(
        "[{}] L({:6},{:6})->({:+.2},{:+.2})  R({:6},{:6})->({:+.2},{:+.2})  LT {:.2}  RT {:.2}  {}",
        slot + 1,
        state.left_stick.0,
        state.left_stick.1,
        lx,
        ly,
        state.right_stick.0,
        state.right_stick.1,
        rx,
        ry,
        lt,
        rt,
        buttons.join(" "),
    );
    let _ = std::io::stdout().flush();
}

/// Borra la línea actual de la consola para redibujarla en el sitio.
fn clear_line() {
    print!("\r\x1b[2K");
}
