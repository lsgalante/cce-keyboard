//! The keymap the virtual keyboard uploads, and the key labels read from it.
//!
//! Compiled from the same RMLVO the compositor's default keymap is: empty
//! names, which libxkbcommon fills from `XKB_DEFAULT_*` and then the system
//! defaults (`cce-compositor` `xkb_config.rs` passes NULL names). So the board
//! labels and types whatever layout the session's hardware keyboard has.

use std::collections::HashMap;

use xkbcommon::xkb;

use crate::layout::Modifier;

/// What a character key prints, plain and shifted. `None` for a key that
/// prints nothing printable (a control character, or nothing at all).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Label {
    pub plain: Option<String>,
    pub shifted: Option<String>,
}

pub struct Keymap {
    /// The keymap in XKB text format, for `zwp_virtual_keyboard_v1.keymap`.
    pub text: String,
    /// Each [`Modifier`]'s mask, indexed by [`Modifier::index`].
    masks: [u32; 4],
    labels: HashMap<u32, Label>,
}

impl Keymap {
    /// Compile the session's default keymap and label `codes` (evdev) from it.
    pub fn compile(codes: &[u32]) -> Option<Self> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(&context, "", "", "", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)?;
        let mut masks = [0; 4];
        for m in Modifier::ALL {
            let index = keymap.mod_get_index(m.xkb_name());
            if index != xkb::MOD_INVALID {
                masks[m.index()] = 1 << index;
            }
        }
        let shift = masks[Modifier::Shift.index()];
        let plain = xkb::State::new(&keymap);
        let mut shifted = xkb::State::new(&keymap);
        shifted.update_mask(shift, 0, 0, 0, 0, 0);
        let printable = |s: String| (!s.is_empty() && !s.chars().any(char::is_control)).then_some(s);
        let labels = codes
            .iter()
            .map(|&code| {
                // xkb keycodes are evdev codes offset by 8.
                let kc = xkb::Keycode::new(code + 8);
                (code, Label { plain: printable(plain.key_get_utf8(kc)), shifted: printable(shifted.key_get_utf8(kc)) })
            })
            .collect();
        Some(Self { text: keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1), masks, labels })
    }

    pub fn label(&self, code: u32) -> Option<&Label> {
        self.labels.get(&code)
    }

    /// The depressed-modifier mask for `mods` held.
    pub fn mask(&self, mods: impl IntoIterator<Item = Modifier>) -> u32 {
        mods.into_iter().fold(0, |acc, m| acc | self.masks[m.index()])
    }
}

impl Label {
    /// The small legend in a key's corner: the shifted symbol, when it is
    /// not simply the capital of the plain one (a digit's `!`, not Q's `Q`).
    pub fn corner(&self) -> Option<&str> {
        let (plain, shifted) = (self.plain.as_deref()?, self.shifted.as_deref()?);
        (shifted != plain && shifted != plain.to_uppercase()).then_some(shifted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_letter_has_no_corner_legend_and_a_digit_does() {
        let q = Label { plain: Some("q".into()), shifted: Some("Q".into()) };
        assert_eq!(q.corner(), None);
        let one = Label { plain: Some("1".into()), shifted: Some("!".into()) };
        assert_eq!(one.corner(), Some("!"));
        let dead = Label { plain: Some("1".into()), shifted: None };
        assert_eq!(dead.corner(), None);
    }
}
