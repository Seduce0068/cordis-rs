//! `ctx.skills`: provider registry for agent skill catalogs.
//!
//! Upstream: `packages/skill/skill/src/index.ts` and
//! `skill-filesystem/src/index.ts` in deepseek-ai/deepseek-harness (MIT),
//! read as untrusted reference data. Providers discover skills; the registry
//! merges their catalogs by rank.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::Result;
use crate::context::Disposer;

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillCatalog {
    pub skills: Vec<Skill>,
}

pub type SkillProvider = Arc<dyn Fn() -> std::result::Result<Vec<Skill>, String> + Send + Sync>;

#[derive(Default)]
struct RegistryState {
    providers: Vec<(u64, i64, SkillProvider)>,
    next_id: u64,
}

/// The shared skill registry.
#[derive(Default, Clone)]
pub struct SkillRegistry {
    state: Arc<RwLock<RegistryState>>,
}

impl SkillRegistry {
    /// Register a provider with a rank (higher rank wins for duplicate names).
    pub fn register_provider(&self, rank: i64, provider: SkillProvider) -> Disposer {
        let state = self.state.clone();
        let mut guard = state.write().expect("skill registry poisoned");
        let id = guard.next_id;
        guard.next_id += 1;
        guard.providers.push((id, rank, provider));
        drop(guard);

        Disposer::new(move || {
            let state = state.clone();
            async move {
                state
                    .write()
                    .expect("skill registry poisoned")
                    .providers
                    .retain(|(current, _, _)| *current != id);
                Ok(())
            }
        })
    }

    /// Merge all provider catalogs, deduplicating by name with rank priority.
    pub fn catalog(&self) -> SkillCatalog {
        let providers = self.state.read().expect("skill registry poisoned");
        let mut by_name: BTreeMap<String, (i64, Skill)> = BTreeMap::new();
        for (_, rank, provider) in providers.providers.iter() {
            for skill in provider().unwrap_or_default() {
                let better = by_name
                    .get(&skill.name)
                    .map(|(current_rank, _)| *rank > *current_rank)
                    .unwrap_or(true);
                if better {
                    by_name.insert(skill.name.clone(), (*rank, skill));
                }
            }
        }
        let mut skills: Vec<Skill> = by_name.into_values().map(|(_, skill)| skill).collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        SkillCatalog { skills }
    }

    pub fn names(&self) -> Vec<String> {
        self.catalog()
            .skills
            .into_iter()
            .map(|skill| skill.name)
            .collect()
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<SkillRegistry> {
    let registry = SkillRegistry::default();
    ctx.provide("skills", registry.clone()).await?;
    Ok(registry)
}
