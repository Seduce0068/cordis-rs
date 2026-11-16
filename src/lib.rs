//! A Rust reimplementation of Cordis, the plugin runtime underneath DSH.
//!
//! Scope of this PoC: service registry, typed events with all five dispatch
//! modes, reversible effects, and fibers with notification-driven reload.
//! Loader/YAML/HMR and Schemastery config validation are out of scope.

pub mod context;
pub mod error;
pub mod events;
pub mod plugin;

pub use context::{Context, Disposer, EffectScope, Lifecycle};
pub use error::{Error, Result};
pub use events::Event;
pub use plugin::{Fiber, FunctionPlugin, Plugin};
