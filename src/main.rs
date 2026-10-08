//! cce-keyboard — an on-screen keyboard.
//!
//! A board of keys on an Overlay-layer surface along the bottom of the
//! screen. The surface takes no keyboard focus, so pressing a key never
//! moves focus off the window being typed into; the key is sent to that
//! window through a `zwp_virtual_keyboard_v1` device (`vkbd.rs`) carrying the
//! session's own keymap (`keymap.rs`), which also labels the character keys.
//!
//! Keys act on press and release with the pointer, so holding one repeats in
//! the focused window like a held hardware key. Shift, Ctrl, Alt and Super
//! latch: a tap applies to the next key, a second tap locks (rim lit), a
//! third releases. Fn latches the same way and swaps the number row to
//! F1–F12 / Del and the arrows to Home / PgDn / PgUp / End (Esc becomes `).
//!
//! One instance per session. `cce-keyboard [toggle|show|hide]` (default
//! `toggle`) forwards to the running board over `cce_ui::ipc::instance`, or
//! becomes it; hiding exits, so a closed board costs nothing — bind
//! `cce-keyboard` in input.kdl to summon it.
//!
//! Config (`~/.config/cce/cce-keyboard/config.kdl`), read at startup:
//! `height 280` (logical px) · `width 0` (0 spans the output) ·
//! `margin 0` (px above the bottom edge) · `reserve true` (windows tile
//! above the board rather than under it). The height is capped to fit the
//! smallest output (`Config::fit`).

mod keymap;
mod layout;
mod vkbd;

use std::sync::Mutex;

use cce_ui::colors::{button_background_color, button_hover_color, button_press_color, control_label_color_u8};
use cce_ui::engine::{
    Application, LayerAnchor, LayerKeyboardInteractivity, LayerKind, LayerSettings, LogicalPosition,
    LogicalSize, WindowSettings,
};
use cce_ui::layout::{button_corner_radius, button_font, button_height, control_gap, parse_font_string, root_plate_inset};
use cce_ui::scene::layout::Rect;
use cce_ui::scene::paint::{AlignH, AlignV, ControlPlate, DisplayList, PaintCtx, PlateStance, TextAttrs, TextLayout};
use cce_ui::scene::Material;
use cce_ui::widget::{ElementState, KeyEvent, MouseButton, MouseScrollDelta};

use keymap::Keymap;
use layout::{Action, KeyDef, KeyRect, Modifier};
use vkbd::VirtualKeyboard;

/// The instance socket: `/tmp/cce-keyboard-<WAYLAND_DISPLAY>.sock`.
const SOCKET_PREFIX: &str = "cce-keyboard";

struct Config {
    height: u32,
    /// 0 spans the output.
    width: u32,
    margin: i32,
    reserve: bool,
}

impl Config {
    fn load() -> Self {
        let mut config = Config { height: 280, width: 0, margin: 0, reserve: true };
        let path = cce_ui::config::get_app_config_path(SOCKET_PREFIX);
        let Ok(text) = std::fs::read_to_string(&path) else { return config };
        let doc = match text.parse::<kdl::KdlDocument>() {
            Ok(doc) => doc,
            Err(e) => {
                log::warn!("[cce-keyboard] {}: {e} — using defaults", path.display());
                return config;
            }
        };
        let value = |name: &str| doc.get(name).and_then(|n| n.entries().first()).map(|e| e.value().clone());
        let int = |name: &str| value(name).and_then(|v| v.as_i64());
        if let Some(h) = int("height") {
            config.height = h.clamp(120, 1200) as u32;
        }
        if let Some(w) = int("width") {
            config.width = w.clamp(0, 8000) as u32;
        }
        if let Some(m) = int("margin") {
            config.margin = m.clamp(0, 2000) as i32;
        }
        if let Some(r) = value("reserve").and_then(|v| v.as_bool()) {
            config.reserve = r;
        }
        config
    }

    /// Fit the board to an output of `(w, h)` logical px. The compositor
    /// closes a layer surface whose exclusive zone leaves less than half the
    /// output (cce-compositor `layer_shell.rs`, river's rule), so the board
    /// never takes more than 45% of the height with its margin; a width
    /// past the output's edge spans it instead.
    fn fit(&mut self, (w, h): (u32, u32)) {
        let most = ((h as f32 * 0.45) as i32 - self.margin).max(60) as u32;
        if self.height > most {
            log::info!("[cce-keyboard] height {} does not fit a {w}x{h} output — using {most}", self.height);
            self.height = most;
        }
        if self.width > w {
            self.width = 0;
        }
    }
}

/// The smallest enabled output's logical size, from `ccectl outputs --json`
/// (one JSON object per line). The board may land on any of them — the
/// compositor picks an output for an unassigned layer surface — so it must
/// fit the smallest.
fn smallest_output() -> Option<(u32, u32)> {
    let ccectl = std::env::var("HOME")
        .map(|home| format!("{home}/.local/bin/ccectl"))
        .ok()
        .filter(|p| std::path::Path::new(p).exists())
        .unwrap_or_else(|| "ccectl".to_string());
    let out = std::process::Command::new(ccectl).args(["outputs", "--json"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|o| o.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true))
        .filter_map(|o| Some((o.get("logical_w")?.as_u64()? as u32, o.get("logical_h")?.as_u64()? as u32)))
        .filter(|&(w, h)| w > 0 && h > 0)
        .min_by_key(|&(w, h)| w as u64 * h as u64)
}

/// A latching key's state: off, for the next key, or until tapped again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Latch {
    #[default]
    Off,
    Once,
    Locked,
}

impl Latch {
    fn tapped(self) -> Self {
        match self {
            Latch::Off => Latch::Once,
            Latch::Once => Latch::Locked,
            Latch::Locked => Latch::Off,
        }
    }

    /// After a key has been sent: a one-shot latch is used up.
    fn spent(self) -> Self {
        if self == Latch::Once {
            Latch::Off
        } else {
            self
        }
    }

    fn on(self) -> bool {
        self != Latch::Off
    }
}

/// The key the pointer is holding down, and what pressing it sent.
struct Held {
    at: (usize, usize),
    action: Action,
    /// The modifiers pressed around it, released after it in reverse.
    mods: Vec<Modifier>,
}

#[derive(Debug, Clone)]
enum Message {
    Hide,
}

/// The keymap and keyboard `main` set up before the runner starts, for
/// `KeyboardApp::new` (`engine::run` takes no arguments).
static PARKED: Mutex<Option<(Keymap, VirtualKeyboard)>> = Mutex::new(None);

struct KeyboardApp {
    config: Config,
    rows: Vec<Vec<KeyDef>>,
    keymap: Keymap,
    vk: VirtualKeyboard,
    /// Indexed by [`Modifier::index`].
    latches: [Latch; 4],
    fn_latch: Latch,
    held: Option<Held>,
    hover: Option<(usize, usize)>,
    size: (f32, f32),
}

impl KeyboardApp {
    fn geometry(&self) -> (Vec<Vec<KeyRect>>, f32) {
        let inset = root_plate_inset();
        let (w, h) = self.size;
        let area = (inset, inset, (w - 2.0 * inset).max(0.0), (h - 2.0 * inset).max(0.0));
        // The ladder's control gap spaces a row of buttons; a board is a
        // dense grid of them, where that gap would eat a third of each key.
        // Never wider than it, but no more than a share of the row pitch.
        let gap = control_gap().min(area.3 / self.rows.len().max(1) as f32 * 0.15);
        (layout::place(&self.rows, area, gap), gap)
    }

    fn key_at(&self, pos: LogicalPosition) -> Option<(usize, usize)> {
        let (rects, gap) = self.geometry();
        layout::hit(&rects, gap, pos.x, pos.y)
    }

    fn shift_on(&self) -> bool {
        self.latches[Modifier::Shift.index()].on()
    }

    fn press(&mut self, at: (usize, usize)) {
        let action = self.rows[at.0][at.1].action(self.fn_latch.on());
        let mut mods = Vec::new();
        match action {
            Action::Mod(m) => self.latches[m.index()] = self.latches[m.index()].tapped(),
            Action::Fn => self.fn_latch = self.fn_latch.tapped(),
            Action::Hide => {}
            Action::Code(code) => {
                mods = Modifier::ALL.into_iter().filter(|m| self.latches[m.index()].on()).collect();
                for m in &mods {
                    self.vk.key(m.code(), true);
                }
                if !mods.is_empty() {
                    self.vk.modifiers(self.keymap.mask(mods.iter().copied()));
                }
                self.vk.key(code, true);
            }
        }
        self.held = Some(Held { at, action, mods });
    }

    /// Let go of the held key; `over` is the key under the pointer now.
    fn release(&mut self, over: Option<(usize, usize)>) -> Option<Message> {
        let held = self.held.take()?;
        match held.action {
            Action::Code(code) => {
                self.vk.key(code, false);
                for m in held.mods.iter().rev() {
                    self.vk.key(m.code(), false);
                }
                if !held.mods.is_empty() {
                    self.vk.modifiers(0);
                }
                for latch in &mut self.latches {
                    *latch = latch.spent();
                }
                self.fn_latch = self.fn_latch.spent();
                None
            }
            // Only a release still on the key closes the board: sliding off
            // is the way to change your mind.
            Action::Hide => (over == Some(held.at)).then_some(Message::Hide),
            Action::Mod(_) | Action::Fn => None,
        }
    }

    fn paint_key(&self, pc: &mut PaintCtx, at: (usize, usize), r: KeyRect, row_h: f32) {
        let action = self.rows[at.0][at.1].action(self.fn_latch.on());
        let latch = match action {
            Action::Mod(m) => self.latches[m.index()],
            Action::Fn => self.fn_latch,
            _ => Latch::Off,
        };
        let down = self.held.as_ref().is_some_and(|h| h.at == at) || latch.on();
        let face = if down {
            button_press_color()
        } else if self.hover == Some(at) {
            button_hover_color()
        } else {
            button_background_color()
        };
        let rect = Rect { x: r.x, y: r.y, width: r.w, height: r.h };
        // A key is a keycap: a raised plate that sinks flush while it is
        // down — held by the pointer, or a latched modifier. A locked latch
        // also lights the plate's own rim, the focus-ring treatment.
        let stance = if down { PlateStance::Flush } else { PlateStance::Raised };
        let tint = (latch == Latch::Locked).then(ControlPlate::focus_tint);
        let plate = ControlPlate::control(rect, button_corner_radius(), stance, Material::face(face));
        pc.control_plate(&plate.with_tint(tint));

        let (family, base) = parse_font_string(&button_font());
        let size = base.unwrap_or(14.0) * (row_h / button_height().max(1.0)).clamp(1.0, 2.0);
        let color = control_label_color_u8();
        let centered = TextLayout {
            wrap_width: Some(rect.width),
            box_height: rect.height,
            align_h: AlignH::Center,
            align_v: AlignV::Middle,
        };

        if let Some((name, icon)) = layout::fixed_label(action) {
            let glyph = icon.and_then(|icon| cce_ui::upload_icon(icon, 64));
            if let Some((image, iw, ih)) = glyph {
                let s = (rect.width.min(rect.height) * 0.45).max(4.0);
                let (iw, ih) = (iw as f32, ih as f32);
                let (dw, dh) = if iw >= ih { (s, s * ih / iw.max(1.0)) } else { (s * iw / ih.max(1.0), s) };
                let at = Rect {
                    x: rect.x + (rect.width - dw) / 2.0,
                    y: rect.y + (rect.height - dh) / 2.0,
                    width: dw,
                    height: dh,
                };
                pc.image(image, at, 1.0);
            } else {
                pc.text_boxed(name, rect.x, rect.y, size * 0.8, color, Some(family), None, TextAttrs::default(), centered);
            }
            return;
        }

        let Action::Code(code) = action else { return };
        let Some(label) = self.keymap.label(code) else { return };
        let shift = self.shift_on();
        let main = if shift { label.shifted.as_deref().or(label.plain.as_deref()) } else { label.plain.as_deref() };
        if let Some(main) = main {
            pc.text_boxed(main, rect.x, rect.y, size, color, Some(family.clone()), None, TextAttrs::default(), centered);
        }
        if !shift {
            if let Some(corner) = label.corner() {
                let small = size * 0.6;
                let pad = (rect.height * 0.12).max(2.0);
                let bounds = Some([rect.x, rect.y, rect.x + rect.width, rect.y + rect.height]);
                pc.text_faded(corner, rect.x + pad, rect.y + pad * 0.5, small, color, 0.55, Some(family), bounds);
            }
        }
    }
}

impl Application for KeyboardApp {
    type Message = Message;

    fn create(sender: cce_ui::engine::AppSender<Self::Message>) -> Self {
        // The app keeps calloop's sender; `AppSender` converts into it.
        let sender: calloop::channel::Sender<Self::Message> = sender.into();
        let (keymap, vk) = PARKED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .expect("main parks the keymap and keyboard before running");
        cce_ui::ipc::instance::serve(move |line| match line.trim() {
            "toggle" | "hide" => {
                sender.send(Message::Hide).ok()?;
                Some("hidden".into())
            }
            "show" => Some("shown".into()),
            other => Some(format!("unknown command {other:?} — toggle, show or hide")),
        });
        let mut config = Config::load();
        if let Some(output) = smallest_output() {
            config.fit(output);
        }
        let size = (config.width.max(1) as f32, config.height as f32);
        Self {
            config,
            rows: layout::rows(),
            keymap,
            vk,
            latches: [Latch::Off; 4],
            fn_latch: Latch::Off,
            held: None,
            hover: None,
            size,
        }
    }

    fn settings(&self) -> WindowSettings {
        WindowSettings {
            title: "Keyboard".to_string(),
            app_id: SOCKET_PREFIX.to_string(),
            width: self.config.width,
            height: self.config.height,
            fullscreen: false,
            min_size: None,
        }
    }

    fn layer(&self) -> Option<LayerSettings> {
        let anchor = if self.config.width == 0 {
            LayerAnchor::BOTTOM | LayerAnchor::LEFT | LayerAnchor::RIGHT
        } else {
            LayerAnchor::BOTTOM
        };
        Some(LayerSettings {
            // Overlay, so the board also stands over a fullscreen window.
            layer: LayerKind::Overlay,
            anchor,
            exclusive_zone: if self.config.reserve { self.config.height as i32 } else { 0 },
            // Never take focus: the window being typed into must keep it.
            keyboard_interactivity: LayerKeyboardInteractivity::None,
            margin: (0, 0, self.config.margin, 0),
            namespace: SOCKET_PREFIX.to_string(),
        })
    }

    fn update(&mut self, msg: Self::Message, _needs_rebuild: &mut bool, exit: &mut bool) {
        match msg {
            Message::Hide => {
                // Give up the socket before the close fade, so a summon
                // during the fade starts a fresh board instead of being
                // answered by this one and dropped with it.
                cce_ui::ipc::instance::cleanup();
                *exit = true;
            }
        }
    }

    fn tick(&mut self, _dt: f32, _needs_rebuild: &mut bool) {}

    fn handle_resize(&mut self, width: f32, height: f32, _scale: f64) {
        self.size = (width, height);
    }

    fn handle_pointer_move(&mut self, pos: LogicalPosition, needs_rebuild: &mut bool) {
        let hover = self.key_at(pos);
        if hover != self.hover {
            self.hover = hover;
            *needs_rebuild = true;
        }
    }

    fn handle_mouse_input(
        &mut self,
        button: MouseButton,
        state: ElementState,
        pos: LogicalPosition,
        needs_rebuild: &mut bool,
    ) -> Option<Self::Message> {
        if button != MouseButton::Left {
            return None;
        }
        *needs_rebuild = true;
        match state {
            ElementState::Pressed => {
                // A second button-down without a release between (a lost
                // release) lets go of the first key before taking the next.
                let _ = self.release(None);
                if let Some(at) = self.key_at(pos) {
                    self.press(at);
                }
                None
            }
            ElementState::Released => self.release(self.key_at(pos)),
        }
    }

    fn handle_mouse_wheel(&mut self, _delta: &MouseScrollDelta, _pos: LogicalPosition, _needs_rebuild: &mut bool) {}

    fn handle_key_input(&mut self, _event: &KeyEvent, _needs_rebuild: &mut bool) -> Option<Self::Message> {
        None
    }

    fn display_list(&mut self, size: LogicalSize, _scale: f64) -> Option<DisplayList> {
        self.size = (size.width, size.height);
        let (rects, _) = self.geometry();
        let mut pc = PaintCtx::new();
        pc.root_plate(size.width, size.height);
        for (r, row) in rects.iter().enumerate() {
            for (i, key) in row.iter().enumerate() {
                self.paint_key(&mut pc, (r, i), *key, key.h);
            }
        }
        Some(pc.finish())
    }

    fn display_list_text(&self) -> bool {
        true
    }

    fn on_exit(&mut self) {
        // Nothing may stay pressed in the seat once the board is gone; the
        // keyboard's own Drop releases whatever this misses.
        let _ = self.release(None);
    }
}

fn main() {
    env_logger::init();
    let command = std::env::args().nth(1).unwrap_or_else(|| "toggle".to_string());
    match command.as_str() {
        "toggle" | "show" | "hide" => {}
        "-h" | "--help" | "help" => {
            println!("usage: cce-keyboard [toggle|show|hide]   (default toggle)");
            return;
        }
        other => {
            eprintln!("cce-keyboard: unknown command {other:?} — toggle, show or hide");
            std::process::exit(2);
        }
    }
    if cce_ui::ipc::instance::forward_or_claim(SOCKET_PREFIX, &command) {
        return;
    }
    // No board was up: there is nothing to hide.
    if command == "hide" {
        cce_ui::ipc::instance::cleanup();
        return;
    }
    let codes = layout::all_codes(&layout::rows());
    let Some(keymap) = Keymap::compile(&codes) else {
        eprintln!("cce-keyboard: could not compile the session keymap");
        cce_ui::ipc::instance::cleanup();
        std::process::exit(1);
    };
    let vk = match VirtualKeyboard::connect(&keymap.text) {
        Ok(vk) => vk,
        Err(e) => {
            eprintln!("cce-keyboard: {e}");
            cce_ui::ipc::instance::cleanup();
            std::process::exit(1);
        }
    };
    *PARKED.lock().unwrap_or_else(|e| e.into_inner()) = Some((keymap, vk));
    cce_ui::engine::run::<KeyboardApp>();
    cce_ui::ipc::instance::cleanup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_board_leaves_the_compositor_half_the_output() {
        let mut config = Config { height: 280, width: 0, margin: 0, reserve: true };
        config.fit((640, 360));
        assert_eq!(config.height, 162);
        let mut config = Config { height: 280, width: 2000, margin: 20, reserve: true };
        config.fit((1440, 900));
        assert_eq!((config.height, config.width), (280, 0));
        let mut config = Config { height: 400, width: 800, margin: 20, reserve: true };
        config.fit((1440, 900));
        assert_eq!((config.height, config.width), (385, 800));
    }

    #[test]
    fn a_latch_cycles_once_locked_off() {
        let l = Latch::Off.tapped();
        assert_eq!(l, Latch::Once);
        assert_eq!(l.tapped(), Latch::Locked);
        assert_eq!(l.tapped().tapped(), Latch::Off);
    }

    #[test]
    fn only_a_one_shot_latch_is_spent_by_a_key() {
        assert_eq!(Latch::Once.spent(), Latch::Off);
        assert_eq!(Latch::Locked.spent(), Latch::Locked);
        assert_eq!(Latch::Off.spent(), Latch::Off);
    }
}
