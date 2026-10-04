//! The key table and its geometry: pure data and arithmetic, no Wayland.
//!
//! Five rows of 15 units each, a US ANSI board with the Caps Lock position
//! given to Fn. Key codes are evdev (`linux/input-event-codes.h`), which is
//! what `zwp_virtual_keyboard_v1.key` takes; the labels of character keys are
//! not here at all — they are read from the uploaded keymap (`keymap.rs`), so
//! a non-US layout labels itself.

/// Every row is this many units wide.
pub const ROW_UNITS: f32 = 15.0;

/// evdev key codes the board sends.
pub mod code {
    pub const ESC: u32 = 1;
    pub const N1: u32 = 2;
    pub const N0: u32 = 11;
    pub const MINUS: u32 = 12;
    pub const EQUAL: u32 = 13;
    pub const BACKSPACE: u32 = 14;
    pub const TAB: u32 = 15;
    pub const Q: u32 = 16;
    pub const P: u32 = 25;
    pub const LEFTBRACE: u32 = 26;
    pub const RIGHTBRACE: u32 = 27;
    pub const ENTER: u32 = 28;
    pub const LEFTCTRL: u32 = 29;
    pub const A: u32 = 30;
    pub const L: u32 = 38;
    pub const SEMICOLON: u32 = 39;
    pub const APOSTROPHE: u32 = 40;
    pub const GRAVE: u32 = 41;
    pub const LEFTSHIFT: u32 = 42;
    pub const BACKSLASH: u32 = 43;
    pub const Z: u32 = 44;
    pub const SLASH: u32 = 53;
    pub const LEFTALT: u32 = 56;
    pub const SPACE: u32 = 57;
    pub const F1: u32 = 59;
    pub const F11: u32 = 87;
    pub const F12: u32 = 88;
    pub const HOME: u32 = 102;
    pub const UP: u32 = 103;
    pub const PAGEUP: u32 = 104;
    pub const LEFT: u32 = 105;
    pub const RIGHT: u32 = 106;
    pub const END: u32 = 107;
    pub const DOWN: u32 = 108;
    pub const PAGEDOWN: u32 = 109;
    pub const DELETE: u32 = 111;
    pub const LEFTMETA: u32 = 125;
}

/// A modifier the board latches. Order is press order around a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    Ctrl,
    Alt,
    Super,
    Shift,
}

impl Modifier {
    pub const ALL: [Modifier; 4] = [Modifier::Ctrl, Modifier::Alt, Modifier::Super, Modifier::Shift];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The key pressed for it (always the left one).
    pub fn code(self) -> u32 {
        match self {
            Modifier::Ctrl => code::LEFTCTRL,
            Modifier::Alt => code::LEFTALT,
            Modifier::Super => code::LEFTMETA,
            Modifier::Shift => code::LEFTSHIFT,
        }
    }

    /// The xkb modifier name its mask is looked up by.
    pub fn xkb_name(self) -> &'static str {
        match self {
            Modifier::Ctrl => "Control",
            Modifier::Alt => "Mod1",
            Modifier::Super => "Mod4",
            Modifier::Shift => "Shift",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Modifier::Ctrl => "Ctrl",
            Modifier::Alt => "Alt",
            Modifier::Super => "Super",
            Modifier::Shift => "Shift",
        }
    }
}

/// What a key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Send this evdev code, held as long as the pointer holds the key.
    Code(u32),
    /// Latch a modifier for the next key; a second tap locks it.
    Mod(Modifier),
    /// Latch the Fn layer: the keys with a second action switch to it.
    Fn,
    /// Close the keyboard.
    Hide,
}

/// One key: its action, its action on the Fn layer, and its width in units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyDef {
    pub action: Action,
    pub fn_action: Option<Action>,
    pub units: f32,
}

impl KeyDef {
    /// The action in force with the Fn layer on or off.
    pub fn action(&self, fn_layer: bool) -> Action {
        match (fn_layer, self.fn_action) {
            (true, Some(a)) => a,
            _ => self.action,
        }
    }
}

const fn key(code: u32) -> KeyDef {
    KeyDef { action: Action::Code(code), fn_action: None, units: 1.0 }
}

const fn wide(action: Action, units: f32) -> KeyDef {
    KeyDef { action, fn_action: None, units }
}

const fn layered(code: u32, fn_code: u32, units: f32) -> KeyDef {
    KeyDef { action: Action::Code(code), fn_action: Some(Action::Code(fn_code)), units }
}

/// The board, top row first.
pub fn rows() -> Vec<Vec<KeyDef>> {
    use code::*;
    let run = |from: u32, to: u32| (from..=to).map(key).collect::<Vec<_>>();
    let mut number = vec![layered(ESC, GRAVE, 1.0)];
    number.extend((N1..=N0).map(|c| layered(c, F1 + (c - N1), 1.0)));
    number.extend([layered(MINUS, F11, 1.0), layered(EQUAL, F12, 1.0), layered(BACKSPACE, DELETE, 2.0)]);

    let mut top = vec![wide(Action::Code(TAB), 1.5)];
    top.extend(run(Q, P));
    top.extend([key(LEFTBRACE), key(RIGHTBRACE), wide(Action::Code(BACKSLASH), 1.5)]);

    let mut home = vec![wide(Action::Fn, 1.75)];
    home.extend(run(A, L));
    home.extend([key(SEMICOLON), key(APOSTROPHE), wide(Action::Code(ENTER), 2.25)]);

    let mut bottom = vec![wide(Action::Mod(Modifier::Shift), 2.25)];
    bottom.extend(run(Z, SLASH));
    bottom.push(wide(Action::Mod(Modifier::Shift), 2.75));

    let space = vec![
        wide(Action::Mod(Modifier::Ctrl), 1.5),
        wide(Action::Mod(Modifier::Super), 1.25),
        wide(Action::Mod(Modifier::Alt), 1.25),
        wide(Action::Code(SPACE), 6.0),
        layered(LEFT, HOME, 1.0),
        layered(DOWN, PAGEDOWN, 1.0),
        layered(UP, PAGEUP, 1.0),
        layered(RIGHT, END, 1.0),
        wide(Action::Hide, 1.0),
    ];
    vec![number, top, home, bottom, space]
}

/// Every code the board can send, for the keymap's label pass.
pub fn all_codes(rows: &[Vec<KeyDef>]) -> Vec<u32> {
    let mut codes: Vec<u32> = rows
        .iter()
        .flatten()
        .flat_map(|k| [Some(k.action), k.fn_action])
        .flatten()
        .filter_map(|a| match a {
            Action::Code(c) => Some(c),
            _ => None,
        })
        .collect();
    codes.sort_unstable();
    codes.dedup();
    codes
}

/// A key that is not a character: the name it wears, or the bundled
/// cce-icons glyph standing in for it (with the name as the fallback).
pub fn fixed_label(action: Action) -> Option<(&'static str, Option<&'static str>)> {
    use code::*;
    let named = |name| Some((name, None));
    match action {
        Action::Mod(m) => named(m.label()),
        Action::Fn => named("Fn"),
        Action::Hide => Some(("Hide", Some("chevron-down"))),
        Action::Code(c) => match c {
            ESC => named("Esc"),
            BACKSPACE => named("Back"),
            DELETE => named("Del"),
            TAB => named("Tab"),
            ENTER => named("Enter"),
            SPACE => named(""),
            LEFT => Some(("←", Some("arrow-left"))),
            RIGHT => Some(("→", Some("arrow-right"))),
            UP => Some(("↑", Some("arrow-up"))),
            DOWN => Some(("↓", Some("arrow-down"))),
            HOME => named("Home"),
            END => named("End"),
            PAGEUP => named("PgUp"),
            PAGEDOWN => named("PgDn"),
            F1..=68 => FN_NAMES.get((c - F1) as usize).map(|n| (*n, None)),
            F11 => named("F11"),
            F12 => named("F12"),
            _ => None,
        },
    }
}

const FN_NAMES: [&str; 10] = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10"];

/// A key's rectangle, in the surface's logical px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Where every key sits inside `area` (x, y, w, h), keys `gap` apart.
///
/// Units are a pitch, not a width: a key `u` units wide spans `u` pitches
/// less one gap, so the columns line up from row to row the way a real
/// board's do, whatever the gap.
pub fn place(rows: &[Vec<KeyDef>], area: (f32, f32, f32, f32), gap: f32) -> Vec<Vec<KeyRect>> {
    let (ax, ay, aw, ah) = area;
    let n = rows.len().max(1) as f32;
    let pitch_x = (aw + gap) / ROW_UNITS;
    let pitch_y = (ah + gap) / n;
    rows.iter()
        .enumerate()
        .map(|(r, row)| {
            let mut at = 0.0;
            row.iter()
                .map(|k| {
                    let rect = KeyRect {
                        x: ax + at * pitch_x,
                        y: ay + r as f32 * pitch_y,
                        w: (k.units * pitch_x - gap).max(0.0),
                        h: (pitch_y - gap).max(0.0),
                    };
                    at += k.units;
                    rect
                })
                .collect()
        })
        .collect()
}

/// The key under (`x`, `y`), as (row, index). The gaps belong to the keys
/// on either side of them — half each — so a press between two keys is
/// never lost; only a press outside the board misses.
pub fn hit(rects: &[Vec<KeyRect>], gap: f32, x: f32, y: f32) -> Option<(usize, usize)> {
    let half = gap / 2.0;
    rects.iter().enumerate().find_map(|(r, row)| {
        row.iter()
            .position(|k| x >= k.x - half && x < k.x + k.w + half && y >= k.y - half && y < k.y + k.h + half)
            .map(|i| (r, i))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_fifteen_units() {
        for (r, row) in rows().iter().enumerate() {
            let units: f32 = row.iter().map(|k| k.units).sum();
            assert!((units - ROW_UNITS).abs() < 1e-4, "row {r} is {units} units");
        }
    }

    #[test]
    fn rows_fill_the_area_edge_to_edge() {
        let rows = rows();
        let rects = place(&rows, (10.0, 20.0, 1000.0, 300.0), 6.0);
        for row in &rects {
            let first = row.first().unwrap();
            let last = row.last().unwrap();
            assert!((first.x - 10.0).abs() < 1e-3);
            assert!((last.x + last.w - 1010.0).abs() < 1e-3, "row ends at {}", last.x + last.w);
        }
        let last = rects.last().unwrap()[0];
        assert!((last.y + last.h - 320.0).abs() < 1e-3);
    }

    #[test]
    fn neighbours_are_one_gap_apart() {
        let rows = rows();
        let rects = place(&rows, (0.0, 0.0, 900.0, 250.0), 8.0);
        for row in &rects {
            for pair in row.windows(2) {
                assert!((pair[1].x - (pair[0].x + pair[0].w) - 8.0).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn a_press_in_a_gap_lands_on_a_neighbour() {
        let rows = rows();
        let gap = 10.0;
        let rects = place(&rows, (0.0, 0.0, 1500.0, 500.0), gap);
        let q = rects[1][1];
        // Just right of Q, inside the gap: still Q.
        assert_eq!(hit(&rects, gap, q.x + q.w + 4.0, q.y + 5.0), Some((1, 1)));
        // Just left of W, inside the same gap: W.
        assert_eq!(hit(&rects, gap, q.x + q.w + 6.0, q.y + 5.0), Some((1, 2)));
        assert_eq!(hit(&rects, gap, -20.0, 5.0), None);
    }

    #[test]
    fn the_fn_layer_swaps_only_layered_keys() {
        let rows = rows();
        assert_eq!(rows[0][1].action(true), Action::Code(code::F1));
        assert_eq!(rows[0][10].action(true), Action::Code(code::F1 + 9));
        assert_eq!(rows[0][13].action(true), Action::Code(code::DELETE));
        assert_eq!(rows[1][1].action(true), Action::Code(code::Q));
        assert_eq!(rows[4][4].action(false), Action::Code(code::LEFT));
        assert_eq!(rows[4][4].action(true), Action::Code(code::HOME));
    }

    #[test]
    fn every_function_key_is_named() {
        for row in rows() {
            for k in row {
                if let Some(Action::Code(c)) = k.fn_action {
                    assert!(fixed_label(Action::Code(c)).is_some() || c == code::GRAVE, "code {c} has no label");
                }
            }
        }
    }
}
