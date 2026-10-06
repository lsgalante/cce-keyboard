# cce-keyboard

The on-screen keyboard. Read the workspace guide (`../cce-compositor/WORKSPACE.md`)
first; this file covers only what is particular to this crate.

## Shape

- `src/main.rs` — the `Application`: an Overlay-layer surface anchored to the
  bottom edge, `keyboard_interactivity none`, root plate with one control
  plate per key (raised keycap at rest, flush while held or latched, rim lit
  while locked). Single instance via `cce_ui::ipc::instance`;
  `cce-keyboard [toggle|show|hide]`, and hiding exits.
- `src/layout.rs` — the key table (evdev codes, unit widths, the Fn layer)
  and the pure geometry (`place`, `hit`). Unit-tested; no Wayland.
- `src/keymap.rs` — compiles the session's default keymap with
  libxkbcommon (empty RMLVO, the same defaults `cce-compositor`'s
  `xkb_config.rs` uses) and reads the character keys' labels from it.
- `src/vkbd.rs` — `zwp_virtual_keyboard_v1` on a **second Wayland
  connection** of its own. The protocol has no events, so nothing needs
  dispatching, and keeping it off the runner's `EngineState` means cce-ui
  needed no change.

## Things that are deliberate

- **No key repeat here.** The compositor gives virtual keyboards repeat info
  (`keyboard.rs` `DEFAULT_REPEAT_*`) and the focused client repeats a held
  key itself; the board just keeps the key down while the pointer does.
- **Modifiers are pressed lazily**, around the next key, not when latched.
  A latched Super must not hold the compositor in its Super-held adjust mode.
  Both the modifier key events and an explicit `modifiers` mask are sent.
- **The board fits the smallest output** (`Config::fit`, via `ccectl outputs
  --json`): the compositor *destroys* a layer surface whose exclusive zone
  leaves less than half the output (`layer_shell.rs`, river's rule). A
  280px board on a 360px-tall scale-2 shadow output vanished on map before
  this.
- **The key gap is capped at 15% of the row pitch**, never wider than the
  ladder's `control_gap()`: the ladder spaces rows of buttons, and on a
  dense grid its gap ate a third of every key.

## Shown by a touched field

The compositor runs `cce-keyboard show` when a touch activates a
text-input-v3 field and `cce-keyboard hide` when that field lets go
(`cce-compositor`'s `osk.rs`; `window_manager { osk_on_touch }`). cce-ui
widgets announce their fields through `cce_ui::text_input::claim`. The board
does not have to do anything for this: it stays a virtual keyboard, and a
tap on it does not take focus, so the field stays enabled while it types.

## Depends on a compositor fix

A click on a layer surface used to give it keyboard focus whatever its
`keyboard_interactivity` (`cursor.rs`, button and touch paths), which took
focus off the window being typed into on the first key. Fixed by
`layer_takes_click_focus` in cce-compositor; a session on an older `cce-fx`
shows the board, latches modifiers, and types nothing.

## Verifying

Shadow only (never run a build from the plain shell):

```sh
cce-shadow start --new --bin /abs/path/target/release/cce-fx
cce-shadow spawn /home/lsgalante/.local/bin/cce-text-editor
cce-shadow spawn env CCE_FONTS_DIR=… CCE_ICONS_DIR=… /abs/path/target/release/cce-keyboard
cce-shadow ctl pointer-move-to <x> <y>; cce-shadow ctl pointer-click
```

Check `ctl windows` still shows the editor `focused=true` after clicking keys,
and do a `--scale 2` pass (pointer coords are logical: shot px ÷ 2).
