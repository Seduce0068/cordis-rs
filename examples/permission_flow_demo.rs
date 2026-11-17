//! Demo: permission preset switching controls tool execution.
//!
//! Wires `ctx.permissionPresets` → `ctx.approval` → `ctx.tools` pipeline and
//! shows how the same tool call is allowed/denied under different presets.

use std::sync::Arc;

use cordis_rs::Context;
use cordis_rs::core_services::{
    ApprovalDecision, ToolDefinition, ToolError, ToolExecContext, ToolOutput, provide_approval,
    provide_permission_presets, provide_tools,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let ctx = Context::root("permission-flow");
        let (registry, pipeline) = provide_tools(&ctx).await?;
        let approval = provide_approval(&ctx).await?;
        let presets = provide_permission_presets(&ctx).await?;

        presets.bind_approval(approval.clone());
        pipeline.with_approval(approval.clone());

        // An answerer that allows when asked (danger-full-access / ask).
        let _answerer = approval.add_answerer(
            "demo-user",
            Arc::new(|request| {
                if request.action == "tool:bash" {
                    Some(ApprovalDecision::Allow)
                } else {
                    None
                }
            }),
        );

        let _tool = registry.register(ToolDefinition::new(
            "bash",
            "Run a command",
            Arc::new(|_args, _exec| {
                Box::pin(async { Ok(ToolOutput::text("command output")) })
                    as cordis_rs::events::BoxFuture<'static, Result<ToolOutput, ToolError>>
            }),
        ));

        let call = || async {
            pipeline
                .run(
                    &registry,
                    "bash",
                    serde_json::json!({}),
                    ToolExecContext::agent("demo"),
                )
                .await
        };

        for preset in ["read-only", "workspace-write", "danger-full-access"] {
            let state = presets.switch(preset)?;
            let result = call().await;
            let label = match result {
                Ok(output) => format!("allowed: {}", output.rendered.join("")),
                Err(error) => format!("denied: {error}"),
            };
            println!(
                "{preset}: approval={} sandbox={} -> {label}",
                state.approval.as_str(),
                state.sandbox.as_str()
            );
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}
