# Input/output examples

Six demos of frost's input and audio output: keyboard, mouse, and gamepad
state read through the `Context` accessors, and one-shot/looping playback
through `frost::Audio` and `frost::Sound`.

| Example | What it shows | Run with |
|---|---|---|
| `player` | keyboard: `W`/`A`/`S`/`D` move a sprite relative to the way it faces at 100 px/s and `Q`/`E` spin it at 360°/s while held — the whole input model in eight lines of `key_down` | `cargo run --example player` |
| `cursor` | mouse: a watering-can sprite follows the pointer (`Context::mouse_position`); holding the left button turns the can a quarter turn over half a second (`Context::mouse_button_down`), and at full tilt water pours out of the spout as drops | `cargo run --example cursor` |
| `cone_collider` | keyboard **and** gamepad: the same `W`/`A`/`S`/`D` + `Q`/`E` as `player`, plus a connected gamepad's left stick walking (up forward, left/right strafing) and right stick turning; the torch hall of the `cone` example with the walls pushing the player back | `cargo run --example cone_collider` |
| `sound` | audio out: the swoosh decoded once into a `frost::Sound`, played through a `frost::Audio` — held Space re-triggers it as overlapping one-shots (each press pulses the center circle), L starts or stops the single loop (a wobbling ring behind it), held `+`/`-` ramp the master volume by tenths (bar at the bottom), Esc quits | `cargo run --example sound` |
| `audio_test` | the smoke test under the whole module: no frost, no window — it opens rodio's default sink, plays `assets/audio/swoof.wav` once at a low volume, sleeps, and exits | `cargo run --example audio_test` |
| `torch` | mouse driving a particle demo: the torch chases the cursor while it is inside the window and wanders on its own until then; its local-space embers ride every move of the torch, and user-space sparks are kicked off the tip when it moves fast | `cargo run --example torch` |

The six files sit in this folder rather than at the examples' top level,
so each one is declared explicitly in `Cargo.toml` (cargo only
auto-discovers `examples/*.rs` and `examples/*/main.rs`); the example
names are unchanged. `torch` is also a particles demo — for the particle
batch API it uses, see `../particles/README.md`.

## The model

### Input: held state, polled per frame

Frost reports *state*, not events: the `Context` accessors tell you what
is down right now, updated as input events arrive since the previous
frame.

- `key_down(key)` — whether the physical key is currently held.
- `mouse_position()` — the cursor in user coordinates (origin at the
  window's center, y up), or `None` when it is outside the window — after
  it has left, or before it first moved in. The last known position is no
  longer reported, so a demo that wants the pointer to stick keeps one
  itself.
- `mouse_button_down(button)` — whether the button is held. The held
  state is dropped when the cursor leaves the window or the window loses
  focus, so a button can never appear stuck down.
- `gamepads()` — the connected gamepads (as `gilrs::Gamepad`s) in the
  order they connected; empty when none is connected, or when the
  platform's input devices could not be opened at startup.

The demos all follow the same shape: read the state in `process()` and
integrate it with `dt`. `player` and `cone_collider` collapse two held
keys into one axis — `+1` if only the positive key is down, `-1` if only
the negative one, `0` if both or neither — and move or turn by
`axis * speed * dt`, which is why pressing `W` and `S` at once stands
still. Because frost gives you "is it down", not "was it just pressed", a
demo that wants to fire once per press keeps its own previous state;
`sound` instead fires *while* the key is down, which is why holding Space
is a rapid-fire of overlapping swooshes and holding `+` ramps the volume
steadily.

### Audio: decode up front, play many times

`Sound::load(path)` (or `Sound::load_bytes`) decodes a file — WAV, FLAC,
MP3, OGG Vorbis, AAC, or M4A — into in-memory f32 samples at creation
time: a missing file or an undecodable format fails with `AudioError`
up front, not at play time, and the samples sit behind an `Arc`, so
re-triggering a sound shares one buffer and never touches the disk again.
`Audio::new()` opens the default output device once (failing with
`AudioError::Device` if there is none); dropping the `Audio` stops
everything it started.

One-shots mix in parallel: `play_once` adds a copy of the sound's
samples to the device's mixer, so the same sound re-triggers as fast as
you like and the copies overlap freely, each running to the end on its
own. The loop is a single sequential player: `play_loop` starts or
*replaces* the one loop (the old one stops, the new one starts over), and
`stop_loop` silences just the loop — one-shots already in the mixer run
out. The master volume, `0.0..=1.0`, applies to the loop live and to
one-shots from the next trigger on, so `set_volume` can be called at any
time. The module is native-only — it is excluded from
`wasm32-unknown-unknown` builds, whose audio backend has no device layer.

## Patterns the examples share

- **The sprite rides the input, not the other way around.** In `player`
  and `cursor` the input sets the node's transform each frame (position
  and facing, or the pointer's position for the can), and everything the
  sprite carries — the water drops, the torch beam — follows for free.
- **A deadzone is what lets inputs mix.** In `cone_collider` a gamepad
  stick inside the deadzone reads as *no input*, not as zero-strength
  input: with the stick centered, the keys still work, and with a key
  held, the stick still works.
- **Assets are pinned to the crate root.** `player`, `cursor`, and
  `sound` load their sprites, font, and wav via `CARGO_MANIFEST_DIR` (a
  compile-time environment variable), so they work from anywhere;
  `audio_test` opens its wav with a path relative to the current working
  directory instead, which is fine for `cargo run` (cargo runs examples
  with the crate root as the working directory), just as `eyes` and
  `eyes_light` do.
- **Audio is set up once, before the scene exists.** Both audio demos
  decode the `Sound` and open the `Audio` in `main`, before `frost::run`,
  and carry the `Audio` in the demo state; on exit, dropping it stops
  everything — no explicit cleanup calls.
