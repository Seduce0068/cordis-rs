//! Reusable Rust port of `@deepseek-ai/dsh-skill-filesystem`.
//!
//! Discovers `SKILL.md` files under configured roots and registers them on the
//! `ctx.skills` provider registry. Source:
//! `packages/skill/skill-filesystem/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::services::skills::{Skill, SkillProvider, SkillRegistry};
use crate::{Context, Result};

#[derive(Debug, Clone)]
pub struct SkillFsConfig {
    /// Discovered skill roots.
    pub roots: Vec<PathBuf>,
    /// Provider rank used when merging catalogs.
    pub rank: i64,
}

impl Default for SkillFsConfig {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            rank: 100,
        }
    }
}

/// Scan one root for `*/SKILL.md` and `<skill>/SKILL.md` bundles.
pub fn discover(root: &Path, source: &str) -> Vec<Skill> {
    let mut skills = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return skills;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skill_md = path.join("SKILL.md");
            if skill_md.is_file() {
                skills.push(skill_from_dir(&path, source));
            }
        } else if path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
            // A flat skill file directly in the root: use the directory name.
            let name = root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("skill")
                .to_string();
            skills.push(Skill {
                name,
                description: read_description(&path),
                path: path.to_string_lossy().to_string(),
                source: source.to_string(),
            });
        }
    }
    skills
}

fn skill_from_dir(path: &Path, source: &str) -> Skill {
    let skill_md = path.join("SKILL.md");
    Skill {
        name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("skill")
            .to_string(),
        description: read_description(&skill_md),
        path: skill_md.to_string_lossy().to_string(),
        source: source.to_string(),
    }
}

fn read_description(path: &Path) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        return String::new();
    };
    // Skip a YAML frontmatter block if present, then take the first non-empty
    // line that is not a heading as the description.
    let mut in_frontmatter = false;
    for (index, line) in content.lines().enumerate() {
        if index == 0 && line.trim() == "---" {
            in_frontmatter = true;
            continue;
        }
        if in_frontmatter {
            if line.trim() == "---" {
                in_frontmatter = false;
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return trimmed.to_string();
    }
    String::new()
}

/// The `dsh-skill-filesystem` plugin. `inject = ['skills']`.
pub struct SkillFilesystem {
    pub config: SkillFsConfig,
}

#[async_trait]
impl crate::Plugin for SkillFilesystem {
    fn name(&self) -> &str {
        "skill-filesystem"
    }

    fn inject(&self) -> &[&str] {
        &["skills"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let registry = ctx.require::<SkillRegistry>("skills").await?;
        let config = self.config.clone();
        ctx.effect(move |scope| {
            let registry = registry.clone();
            let config = config.clone();
            async move {
                let roots = config.roots.clone();
                let rank = config.rank;
                let provider: SkillProvider = Arc::new(move || {
                    let mut skills = Vec::new();
                    for root in &roots {
                        skills.extend(discover(root, "filesystem"));
                    }
                    Ok(skills)
                });
                let disposer = registry.register_provider(rank, provider);
                scope.on_dispose(move || async move { disposer.dispose().await });
                Ok(())
            }
        })
        .await
    }
}

/// Mount with the shared skills seam.
pub async fn mount(ctx: &Context, config: SkillFsConfig) -> Result<SkillRegistry> {
    let registry = if let Some(existing) = ctx.get::<SkillRegistry>("skills") {
        (*existing).clone()
    } else {
        crate::core_services::provide_skills(ctx).await?
    };
    let _ = crate::Fiber::mount_retained(ctx, Box::new(SkillFilesystem { config })).await?;
    Ok(registry)
}
