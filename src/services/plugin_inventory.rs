//! `ctx.pluginInventory`: read-only projection of composition entries.
//!
//! Upstream: `packages/host/plugin-inventory/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! The gateway always reads the current Loader entries (no cache), so
//! enable/disable changes are visible on the next `list()` call.

use std::sync::{Arc, RwLock};

use crate::Result;
use crate::loader::{Composition, CompositionState};

#[derive(Debug, Clone, PartialEq)]
pub struct PluginEntryView {
    pub entry_id: String,
    pub plugin: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginInventorySnapshot {
    pub entries: Vec<PluginEntryView>,
}

#[derive(Default)]
struct InventoryState {
    composition: Composition,
    overrides: CompositionState,
}

#[derive(Default, Clone)]
pub struct PluginInventory {
    state: Arc<RwLock<InventoryState>>,
}

impl PluginInventory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the composition projection (e.g. after loading a cordis.yml).
    pub fn set_composition(&self, composition: Composition) {
        self.state
            .write()
            .expect("plugin inventory poisoned")
            .composition = composition;
    }

    /// Update persisted enable/disable overrides.
    pub fn set_state(&self, state: CompositionState) {
        self.state
            .write()
            .expect("plugin inventory poisoned")
            .overrides = state;
    }

    /// Read the current entries. No runtime cache: reflects the latest state.
    pub fn list(&self) -> PluginInventorySnapshot {
        let guard = self.state.read().expect("plugin inventory poisoned");
        let entries = guard
            .composition
            .entries
            .iter()
            .map(|entry| PluginEntryView {
                entry_id: entry.id.clone(),
                plugin: entry.plugin.clone(),
                enabled: guard.overrides.is_enabled(&entry.id, !entry.disabled),
            })
            .collect();
        PluginInventorySnapshot { entries }
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<PluginInventory> {
    let inventory = PluginInventory::new();
    ctx.provide("pluginInventory", inventory.clone()).await?;
    Ok(inventory)
}
