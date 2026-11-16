//! Demo: agent-loop checkpoints every step into a JSONL session log.
//!
//! Shows the full durable path: scripted agent → session events → checkpoint
//! hook → JSONL file → restore into a fresh Session. No model credentials or
//! network access are required.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cordis_rs::Context;
use cordis_rs::core_services::{
    AgentLoop, CheckpointHook, LlmProvider, Session, StreamChunk, ToolCall, provide_llm,
    provide_tools,
};
use cordis_rs::plugin_ports::mount_session_persistence_jsonl;

fn scripted_provider() -> LlmProvider {
    let step = Arc::new(AtomicUsize::new(0));
    LlmProvider::new(
        "scripted",
        "scripted-1",
        Arc::new(move |_request| {
            let index = step.fetch_add(1, Ordering::SeqCst);
            let text = if index == 0 { "STEP_ONE" } else { "DONE" };
            Box::pin(futures::stream::iter(vec![
                StreamChunk::Text(text.to_string()),
                StreamChunk::Done,
            ])) as Pin<Box<dyn futures::Stream<Item = StreamChunk> + Send + Unpin>>
        }),
    )
}

fn parse_calls(text: &str) -> Vec<ToolCall> {
    // No real tools needed here; the demo focuses on checkpoint durability.
    let _ = text;
    Vec::new()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("cordis-checkpoint-demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let ctx = Context::root("agent-checkpoint-demo");
    let (tools, _) = provide_tools(&ctx).await?;
    let llm = provide_llm(&ctx).await?;
    let store = mount_session_persistence_jsonl(&ctx, &dir).await?;
    let handle = store.create("session-1")?;

    let session = Session::new("session-1", Some("/tmp".to_string()));

    // The checkpoint hook writes only newly committed events. A real host
    // would use a cursor; here the whole log is small, so rewriting is fine.
    let persisted = Arc::new(std::sync::Mutex::new(0usize));
    let hook: CheckpointHook = {
        let handle = handle.clone();
        let persisted = persisted.clone();
        Arc::new(move |s: &Session| {
            let events = s.events();
            // Append only events not yet persisted, identified by seq > count.
            for event in events.iter().skip(*persisted.lock().unwrap()) {
                handle.append(event)?;
            }
            *persisted.lock().unwrap() = events.len();
            Ok(())
        })
    };

    let _provider = llm.register(scripted_provider());
    llm.set_default("scripted");

    let loop_ = AgentLoop::new(llm, tools)
        .with_session(session)
        .with_checkpoint(hook)
        .with_call_parser(Arc::new(parse_calls));
    let run = loop_.run("durable run").await?;

    println!("steps: {}, final: {}", run.steps.len(), run.final_text);

    let events = handle.read()?;
    println!("persisted events: {}", events.len());
    for event in &events {
        println!("  [{}] {}", event.seq, event.event_type);
    }
    assert!(events.iter().any(|event| event.event_type == "turn/start"));
    assert!(events.iter().any(|event| event.event_type == "turn/end"));
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "assistant/message")
    );

    // Restore into a fresh session from disk and compare event counts.
    let restored = store.restore("session-1", Some("/tmp".to_string()))?;
    assert_eq!(restored.seq() as usize, events.len());
    println!("restored session seq: {}", restored.seq());

    std::fs::remove_dir_all(&dir)?;
    Ok(())
}
