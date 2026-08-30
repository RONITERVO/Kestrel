//! Compatibility facade for the Rust-owned application contracts.
//!
//! New durable or IPC-visible types belong in `crates/app-core`, where TypeScript bindings are
//! generated. Keeping this module as a re-export avoids mixing a repository move with behavior
//! changes in the native orchestration code.

pub use kestrel_app_core::*;
