//! A Rust port of the official `@deepseek-ai/dsh-command-compact` plugin.
//!
//! Source (MIT): `packages/compaction/command-compact/src/index.ts` in
//! deepseek-ai/deepseek-harness. The port keeps the plugin's contract:
//! `inject = ['commands', 'compaction']`, an argument-free `/compact`
//! command, and the human-facing outcome strings.
//!
//! NOTE: this is a LOCAL REPRODUCTION with in-test stubs, NOT a test of the
//! production `src/ports/command_compact.rs` (whose result string differs).
//! Production ports are covered by `tests/production_ports.rs`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cordis_rs::{Context, Fiber, Lifecycle, Plugin, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandResult {
    Success {
        text: String,
        source_event_seq: Option<usize>,
    },
    Error {
        text: String,
    },
}

#[derive(Default)]
struct Commands {
    registered: Mutex<Vec<String>>,
}

impl Commands {
    fn register(&self, name: &str) {
        self.registered
            .lock()
            .expect("commands poisoned")
            .push(name.to_string());
    }
}

#[derive(Default, Clone)]
struct Compaction {
    calls: Arc<Mutex<u32>>,
}

impl Compaction {
    async fn compact_now(&self) -> Option<CompactOutcome> {
        *self.calls.lock().expect("compaction poisoned") += 1;
        Some(CompactOutcome {
            shadowed_seqs: 3,
            shadowed_token_count: 128,
            summary_seq: 9,
        })
    }
}

struct CompactOutcome {
    shadowed_seqs: usize,
    shadowed_token_count: usize,
    summary_seq: usize,
}

async fn execute_compact_async(raw_input: &str, compaction: &Compaction) -> CommandResult {
    const USAGE: &str = "Usage: /compact (no arguments)";
    if !raw_input.trim().is_empty() {
        return CommandResult::Error {
            text: USAGE.to_string(),
        };
    }
    match compaction.compact_now().await {
        None => CommandResult::Success {
            text: "No compactable history yet.".to_string(),
            source_event_seq: None,
        },
        Some(result) => CommandResult::Success {
            text: format!(
                "Compacted {} history items (~{} tokens).",
                result.shadowed_seqs, result.shadowed_token_count
            ),
            source_event_seq: Some(result.summary_seq),
        },
    }
}

struct CommandCompact {
    last_result: Arc<Mutex<Option<CommandResult>>>,
}

#[async_trait]
impl Plugin for CommandCompact {
    fn name(&self) -> &str {
        "command-compact"
    }

    fn inject(&self) -> &[&str] {
        &["commands", "compaction"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let commands = ctx.require::<Commands>("commands").await?;
        let compaction = ctx.require::<Arc<Compaction>>("compaction").await?;
        commands.register("/compact");

        let last_result = self.last_result.clone();
        let compaction = compaction.clone();
        ctx.effect(move |_scope| {
            let last_result = last_result.clone();
            let compaction = compaction.clone();
            async move {
                *last_result.lock().expect("result poisoned") =
                    Some(execute_compact_async("", &compaction).await);
                Ok(())
            }
        })
        .await
    }
}

#[tokio::test]
async fn command_compact_runs_after_both_injections_and_reloads() {
    let ctx = Context::root("official-plugin-port");
    let last_result = Arc::new(Mutex::new(None));
    let fiber = Fiber::mount(
        &ctx,
        Box::new(CommandCompact {
            last_result: last_result.clone(),
        }),
    )
    .await
    .unwrap();

    assert_eq!(fiber.state(), Lifecycle::Pending);
    assert!(last_result.lock().unwrap().is_none());

    ctx.provide("commands", Commands::default()).await.unwrap();
    fiber.refresh().await.unwrap();
    assert_eq!(fiber.state(), Lifecycle::Pending);

    let compaction = Arc::new(Compaction::default());
    ctx.provide("compaction", compaction.clone()).await.unwrap();
    fiber.refresh().await.unwrap();
    assert_eq!(fiber.state(), Lifecycle::Active);

    assert_eq!(
        last_result.lock().unwrap().clone(),
        Some(CommandResult::Success {
            text: "Compacted 3 history items (~128 tokens).".to_string(),
            source_event_seq: Some(9),
        })
    );
    assert_eq!(*compaction.calls.lock().unwrap(), 1);

    // Removing one dependency unloads the fiber, as upstream does.
    ctx.remove_service("compaction").unwrap();
    fiber.refresh().await.unwrap();
    assert_eq!(fiber.state(), Lifecycle::Pending);
}
