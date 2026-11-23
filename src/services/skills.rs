//! `ctx.skills`: provider registry for agent skill catalogs.
//!
//! Upstream: `packages/skill/skill/src/index.ts` and
//! `skill-filesystem/src/index.ts` in deepseek-ai/deepseek-harness (MIT),
//! read as untrusted reference data. Providers discover skills; the registry
//! merges their catalogs by rank.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::Result;
use crate::primitives::Disposer;

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: String,
    pub source: String,
}

/// A provider that failed while building the catalog (swarm review R2-ER).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillProviderFailure {
    pub id: u64,
    pub rank: i64,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct SkillCatalog {
    pub skills: Vec<Skill>,
    /// Providers that failed; their skills are absent, so the catalog may be
    /// incomplete. Equality ignores failures for backward compatibility.
    pub failures: Vec<SkillProviderFailure>,
}

impl PartialEq for SkillCatalog {
    fn eq(&self, other: &Self) -> bool {
        self.skills == other.skills
    }
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
        // Snapshot providers under the lock and drop it before invoking any
        // provider callback: a provider may re-enter register_provider and
        // deadlock if the read guard is held (swarm review R8-AP03).
        let providers = {
            let guard = self.state.read().expect("skill registry poisoned");
            guard.providers.clone()
        };
        let mut by_name: BTreeMap<String, (i64, Skill)> = BTreeMap::new();
        let mut failures = Vec::new();
        for (id, rank, provider) in providers.iter() {
            let skills = match provider() {
                Ok(skills) => skills,
                Err(message) => {
                    failures.push(SkillProviderFailure {
                        id: *id,
                        rank: *rank,
                        message: message.escape_debug().to_string(),
                    });
                    Vec::new()
                }
            };
            for skill in skills {
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
        SkillCatalog { skills, failures }
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
    provide_with_disposer(ctx)
        .await
        .map(|(registry, _)| registry)
}

pub async fn provide_with_disposer(
    ctx: &crate::Context,
) -> Result<(SkillRegistry, crate::Disposer)> {
    let registry = SkillRegistry::default();
    let disposer = ctx.provide("skills", registry.clone()).await?;
    Ok((registry, disposer))
}
