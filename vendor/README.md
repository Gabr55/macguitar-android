# Vendored crates

## iced_winit 0.14.0

The [iced_winit](https://github.com/iced-rs/iced) crate of iced 0.14.0, MIT
licensed (its `LICENSE`, from the iced repository), patched for Android,
where iced does not run as released. Every change is marked `MacGuitar
patch`, in `src/lib.rs`, `src/conversion.rs`, `src/window.rs` and
`src/window/state.rs`:

- the event loop is built with the activity given to `android_main`
  (`iced_winit::android::set_app`);
- the events wait for the activity to resume before the window is created;
- the surface is let go of when the activity is suspended, before Android
  takes its native window away (Mali drivers crash on one that outlives it),
  and made again when it resumes;
- the keyboard events are read on Android as on Wasm, winit lacking there
  the modifier supplement;
- a lifted finger leaves no pointer behind, which kept the last control
  touched hovered.

Desktop builds behave as with the released crate.
