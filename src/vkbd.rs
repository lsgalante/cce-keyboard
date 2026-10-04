//! `zwp_virtual_keyboard_v1`: how a key on the board reaches the window that
//! has keyboard focus.
//!
//! It runs on a Wayland connection of its own rather than on the one the
//! cce-ui runner drives the surface with. The protocol is requests only — the
//! compositor never sends the keyboard an event — so this side needs no
//! dispatching, only a flush after each request, and keeping it off the
//! runner's `EngineState` leaves cce-ui untouched. The compositor treats the
//! device like any keyboard (cce-compositor `input_manager.rs`
//! `handle_new_virtual_keyboard`): its keys run the window manager's
//! bindings, and the focused client repeats a held key at the virtual
//! keyboard's repeat info, so the board sends no repeats of its own.

use std::collections::BTreeSet;
use std::io::Write;
use std::os::fd::{AsFd, OwnedFd};

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_keyboard, wl_registry, wl_seat};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

pub struct VirtualKeyboard {
    conn: Connection,
    _queue: EventQueue<Sink>,
    keyboard: ZwpVirtualKeyboardV1,
    /// Codes this side has pressed and not yet released, so a board that
    /// closes mid-press never leaves a key stuck down in the seat.
    down: BTreeSet<u32>,
}

/// The connection's dispatch state: nothing here has events worth reading.
struct Sink;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Sink {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Sink: ignore wl_seat::WlSeat);
delegate_noop!(Sink: ZwpVirtualKeyboardManagerV1);
delegate_noop!(Sink: ZwpVirtualKeyboardV1);

impl VirtualKeyboard {
    /// Connect, create the keyboard on the first seat, and upload `keymap`
    /// (XKB text). The roundtrip at the end surfaces a refusal — a protocol
    /// error on the create or the keymap — here, rather than as a silently
    /// dead connection on the first key.
    pub fn connect(keymap: &str) -> Result<Self, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("no Wayland display: {e}"))?;
        let (globals, mut queue) = registry_queue_init::<Sink>(&conn).map_err(|e| format!("registry: {e}"))?;
        let qh = queue.handle();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|e| format!("wl_seat: {e}"))?;
        let manager: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .map_err(|e| format!("the compositor offers no zwp_virtual_keyboard_manager_v1: {e}"))?;
        let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
        let fd = keymap_fd(keymap).map_err(|e| format!("keymap memfd: {e}"))?;
        // The size counts the terminating NUL the format requires.
        keyboard.keymap(wl_keyboard::KeymapFormat::XkbV1.into(), fd.as_fd(), keymap.len() as u32 + 1);
        queue.roundtrip(&mut Sink).map_err(|e| format!("virtual keyboard refused: {e}"))?;
        Ok(Self { conn, _queue: queue, keyboard, down: BTreeSet::new() })
    }

    /// Press or release `code` (evdev). A release of a key that is not down
    /// is dropped.
    pub fn key(&mut self, code: u32, pressed: bool) {
        if pressed {
            self.down.insert(code);
        } else if !self.down.remove(&code) {
            return;
        }
        let state = if pressed { wl_keyboard::KeyState::Pressed } else { wl_keyboard::KeyState::Released };
        self.keyboard.key(now_ms(), code, state.into());
        self.flush();
    }

    /// The depressed-modifier mask. Sent beside the modifier keys themselves
    /// so the seat's state is stated, not inferred from the key stream.
    pub fn modifiers(&mut self, depressed: u32) {
        self.keyboard.modifiers(depressed, 0, 0, 0);
        self.flush();
    }

    fn flush(&self) {
        if let Err(e) = self.conn.flush() {
            log::error!("[cce-keyboard] virtual keyboard connection lost: {e}");
        }
    }
}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        for code in std::mem::take(&mut self.down).into_iter().rev() {
            self.keyboard.key(now_ms(), code, wl_keyboard::KeyState::Released.into());
        }
        self.keyboard.modifiers(0, 0, 0, 0);
        self.keyboard.destroy();
        let _ = self.conn.flush();
    }
}

/// The keymap in an anonymous file, NUL-terminated, for the compositor to map.
fn keymap_fd(keymap: &str) -> std::io::Result<OwnedFd> {
    let fd = rustix::fs::memfd_create("cce-keyboard-keymap", rustix::fs::MemfdFlags::CLOEXEC)?;
    let mut file = std::fs::File::from(fd);
    file.write_all(keymap.as_bytes())?;
    file.write_all(&[0])?;
    Ok(file.into())
}

/// Key event time: CLOCK_MONOTONIC in ms, the clock input timestamps use.
fn now_ms() -> u32 {
    let t = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    (t.tv_sec as u64 * 1000 + t.tv_nsec as u64 / 1_000_000) as u32
}
