# Generated bindings

This package is generated from Rust types in `crates/app-core` by `ts-rs`, plus the actual
Tauri command signatures and the Rust event registry. `DesktopCommands` and `DesktopEvents`
are the complete native boundary catalog; `defaults.ts` contains Rust-owned values.

Do not hand-edit generated `*.ts` files. Run `npm run bindings:generate` after changing a
Rust-owned contract, then commit the generated diff with the Rust change.

UI contributors use the committed bindings with `npm run ui:check` without compiling Rust.
See [the UI guide](../../apps/desktop/README.md) and [boundary audit](../../docs/UI_BOUNDARIES.md).
