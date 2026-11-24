# BitNet’s Vello CPU adapter for Iced

This Cargo patch replaces the `iced_renderer` package in Iced at `bb97596ddb79c23de7a2393a4671aa64a4146e34`. Iced’s `tiny-skia` feature name is the forwarding slot available at that revision; the adapter maps it to Vello CPU 0.4 and does not link `iced_tiny_skia`. Windowing, UI widgets, input handling and text shaping come from the same Iced revision as the standard BitNet build.

- `src/lib.rs` and `src/fallback.rs` are based on [Iced’s renderer crate at bb97596](https://github.com/iced-rs/iced/tree/bb97596ddb79c23de7a2393a4671aa64a4146e34/renderer). The adapter adds the Vello CPU renderer and preserves its other public exports.
- `src/vello/` is based on [Iced PR #3306 at 3ad0fe9](https://github.com/iced-rs/iced/tree/3ad0fe9402166474cbe8217b439e4901f63bc124/vello_cpu). The port fits this code to the older compositor’s surface initialization and keeps physical damage clipping in the renderer. The Vello CPU single-thread path is used, matching the upstream experiment’s default feature set.
- Both sources are MIT licensed; see `LICENSE`.

On Windows, Vello rasterizes only changed regions into the retained Softbuffer buffer, then copies the whole buffer to the window on every paint request. The window surface itself can be invalidated when restored from the tray even when the retained buffer has not changed.

Each quad draws one shadow under its fill and border. Vello draws the blurred rectangle inside the active clip. Damage includes the offset shadow and the full 2.5σ blur extent, bounded by the layer clip with antialiasing padding. For an opaque window, repainting the background inside the damage clip replaces the previous pixels before drawing translucent content.

Run the renderer’s unit tests from the repository root with `cargo test --locked -p iced_renderer --lib`.
