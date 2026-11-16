//! A Rust reimplementation of Cordis, the plugin runtime underneath DSH.
//!
//! Scope of this PoC: service registry, typed events with all five dispatch
//! modes, reversible effects, fibers with notification-driven reload, plus a
//! scoped config validator and a minimal `cordis.yml` composition loader.
//! Full Schemastery semantics and file-watching HMR remain out of scope.

pub mod config;
pub mod context;
pub mod error;
pub mod events;
pub mod loader;
pub mod plugin;
pub mod ports;

pub use config::{
    FieldSpec, FieldType, ValidationError, ValidationErrors, ValidationReason,
    Value as ConfigValue, validate as validate_config,
};
pub use context::{Context, Disposer, EffectScope, Lifecycle};
pub use error::{Error, Result};
pub use events::Event;
pub use loader::{Composition, Entry, LoaderError, PluginFactory, PluginRegistry, VarTable};
pub use plugin::{Fiber, FunctionPlugin, Plugin};

/// Reusable ports of official DSH plugin implementations.
pub mod plugin_ports {
    pub use crate::ports::tool_todo::{
        Counts, MountedTodo, RawTodo, SessionEvent, SessionProjections, TodoItem, TodoStatus,
        TodoWriteArgs, TodoWriteResult, ToolExecution, ToolFailure, ToolTodo, Tools, describe,
        mount, to_todo_list,
    };
}
