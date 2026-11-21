//! End-to-end demo: agent loop driving real tools through cordis-rs seams.
//!
//! Mounts the LLM, tools, fs, shell and jobs services, registers the tool-fs
//! and tool-bash ports, and runs a scripted agent that writes a file, reads it
//! back, and finishes. No network or model credentials are required: the LLM
//! provider returns scripted tool calls.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cordis_rs::Context;
use cordis_rs::core_services::{
    AgentLoop, LlmProvider, StreamChunk, ToolCall, echo_provider, provide_fs, provide_jobs,
    provide_llm, provide_shell, provide_tools,
};
use cordis_rs::plugin_ports::{ToolBashConfig, ToolFsConfig, mount_tool_bash, mount_tool_fs};

fn scripted_provider() -> LlmProvider {
    let step = Arc::new(AtomicUsize::new(0));
    LlmProvider::new(
        "scripted",
        "scripted-1",
        Arc::new(move |_request| {
            let index = step.fetch_add(1, Ordering::SeqCst);
            let text = match index {
                0 => "WRITE",
                1 => "READ",
                _ => "DONE",
            };
            Box::pin(futures::stream::iter(vec![
                StreamChunk::Text(text.to_string()),
                StreamChunk::Done,
            ])) as Pin<Box<dyn futures::Stream<Item = StreamChunk> + Send + Unpin>>
        }),
    )
}

fn parse_calls(text: &str, path: &str) -> Vec<ToolCall> {
    match text {
        "WRITE" => vec![ToolCall::new(
            "write",
            serde_json::json!({
                "file_path": path,
                "content": "agent wrote this\n",
            }),
        )],
        "READ" => vec![ToolCall::new(
            "read",
            serde_json::json!({ "file_path": path }),
        )],
        _ => Vec::new(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!(
        "cordis-agent-demo-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    let path_str = path.to_string_lossy().to_string();
    let ctx = Context::root("agent-demo");
    let (tools, _) = provide_tools(&ctx).await?;
    let fs = provide_fs(&ctx).await?;
    let shell = provide_shell(&ctx).await?;
    let _jobs = provide_jobs(&ctx).await?;
    let llm = provide_llm(&ctx).await?;

    let _fs_tools = mount_tool_fs(&ctx, fs.clone(), ToolFsConfig::default()).await?;
    let _bash_tools = mount_tool_bash(&ctx, shell, ToolBashConfig::default()).await?;
    let _provider = llm.register(scripted_provider());
    llm.set_default("scripted");

    // An unrelated keyless provider is available for other hosts.
    let _echo = llm.register(echo_provider("echo"));

    let loop_ = AgentLoop::new(llm, tools)
        .with_max_steps(4)
        .with_call_parser(Arc::new(move |text: &str| parse_calls(text, &path_str)));
    let run = loop_.run("create and verify a file").await?;

    println!("steps: {}", run.steps.len());
    for (index, step) in run.steps.iter().enumerate() {
        println!(
            "  step {}: model={}, tool_results={}",
            index + 1,
            step.response.text,
            step.tool_results.len()
        );
        for result in &step.tool_results {
            match result {
                Ok(output) if !output.rendered.is_empty() => {
                    println!("    {}", output.rendered[0].lines().next().unwrap_or(""));
                }
                Ok(_) => {}
                Err(error) => println!("    error: {error}"),
            }
        }
    }

    let content = std::fs::read_to_string(&path)?;
    assert_eq!(content, "agent wrote this\n");
    println!("final text: {}", run.final_text);
    println!("file verified: {content:?}");

    Ok(())
}
