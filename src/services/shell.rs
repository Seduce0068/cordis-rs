//! `ctx.shell`: subprocess execution seam.
//!
//! Upstream: `packages/shell/shell/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! This port resolves a request into a concrete argv and runs it through
//! `std::process::Command`, returning stdout/stderr and an exit status.
//! Sandbox policy is left to the host.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, RwLock};

use crate::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct ShellRequest {
    pub command: String,
    pub cwd: Option<PathBuf>,
    pub timeout_ms: Option<u64>,
    /// Optional per-stream output cap (R3-IV05). None keeps legacy behaviour.
    pub max_output_bytes: Option<usize>,
}

impl ShellRequest {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            cwd: None,
            timeout_ms: None,
            max_output_bytes: None,
        }
    }

    pub fn in_dir(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    pub fn with_max_output_bytes(mut self, limit: usize) -> Self {
        self.max_output_bytes = Some(limit);
        self
    }
}

/// Truncate a stream to `limit` bytes without breaking UTF-8, appending a
/// marker (included in the budget) so callers can see the output was cut.
fn truncate_output(text: &mut String, limit: Option<usize>) {
    let Some(limit) = limit else {
        return;
    };
    const MARKER: &str = "…[truncated]";
    if text.len() <= limit {
        return;
    }
    let budget = limit.saturating_sub(MARKER.len());
    let mut cut = budget.min(text.len());
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text.push_str(MARKER);
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShellSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShellResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShellError {
    InvalidCommand(String),
    Spawn(String),
    TimedOut(u64),
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCommand(message) => write!(f, "invalid command: {message}"),
            Self::Spawn(message) => write!(f, "failed to spawn: {message}"),
            Self::TimedOut(_ms) => write!(f, "command timed out"),
        }
    }
}

impl std::error::Error for ShellError {}

#[derive(Default)]
struct ShellState {
    default_cwd: Option<PathBuf>,
}

/// The shell execution seam.
#[derive(Default, Clone)]
pub struct Shell {
    state: Arc<RwLock<ShellState>>,
}

impl Shell {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_default_cwd(&self, cwd: impl Into<PathBuf>) {
        self.state
            .write()
            .expect("shell state poisoned")
            .default_cwd = Some(cwd.into());
    }

    /// Resolve a request into a concrete program + argv. Uses `sh -c` on
    /// Unix and `cmd /C` on Windows, matching a typical shell executor.
    pub fn resolve(&self, request: &ShellRequest) -> std::result::Result<ShellSpec, ShellError> {
        if request.command.trim().is_empty() {
            return Err(ShellError::InvalidCommand(
                "command must not be empty".to_string(),
            ));
        }
        let cwd = request.cwd.clone().or_else(|| {
            self.state
                .read()
                .expect("shell state poisoned")
                .default_cwd
                .clone()
        });
        #[cfg(windows)]
        let spec = ShellSpec {
            program: "cmd".to_string(),
            args: vec!["/C".to_string(), request.command.clone()],
            cwd,
        };
        #[cfg(not(windows))]
        let spec = ShellSpec {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), request.command.clone()],
            cwd,
        };
        Ok(spec)
    }

    /// Execute a resolved spec synchronously.
    pub fn execute(&self, spec: &ShellSpec) -> std::result::Result<ShellResult, ShellError> {
        let mut command = Command::new(&spec.program);
        command.args(&spec.args);
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let output = command
            .output()
            .map_err(|error| ShellError::Spawn(error.to_string()))?;
        Ok(ShellResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
            success: output.status.success(),
        })
    }

    /// Convenience: resolve and execute in one call.
    pub fn run(&self, request: &ShellRequest) -> std::result::Result<ShellResult, ShellError> {
        let spec = self.resolve(request)?;
        let mut result = self.execute(&spec)?;
        truncate_output(&mut result.stdout, request.max_output_bytes);
        truncate_output(&mut result.stderr, request.max_output_bytes);
        Ok(result)
    }

    /// Resolve and execute with the request's timeout applied. The child is
    /// spawned via `tokio::process` so it can be killed (and reaped) when the
    /// deadline expires — `Command::output()` offers no cancellation (R3-IV02).
    /// `timeout_ms = None` keeps the legacy synchronous path.
    pub async fn run_with_timeout(
        &self,
        request: &ShellRequest,
    ) -> std::result::Result<ShellResult, ShellError> {
        let Some(timeout_ms) = request.timeout_ms else {
            return self.run(request);
        };
        let spec = self.resolve(request)?;
        let mut command = tokio::process::Command::new(&spec.program);
        command.args(&spec.args);
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| ShellError::Spawn(error.to_string()))?;
        // Take the pipes before waiting so `child` stays owned for kill().
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), child.wait()).await
        {
            Ok(Ok(status)) => {
                // Process exited; drain the captured pipes without a deadline
                // (the child is gone, so readers cannot block forever).
                let mut out = Vec::new();
                let mut err = Vec::new();
                // Propagate a pipe read failure instead of returning a
                // partially captured output as if it were complete (swarm
                // review R4-ER07).
                if let Some(mut pipe) = stdout {
                    use tokio::io::AsyncReadExt;
                    pipe.read_to_end(&mut out).await.map_err(|error| {
                        ShellError::Spawn(format!("stdout read failed: {error}"))
                    })?;
                }
                if let Some(mut pipe) = stderr {
                    use tokio::io::AsyncReadExt;
                    pipe.read_to_end(&mut err).await.map_err(|error| {
                        ShellError::Spawn(format!("stderr read failed: {error}"))
                    })?;
                }
                let mut stdout_text = String::from_utf8_lossy(&out).to_string();
                let mut stderr_text = String::from_utf8_lossy(&err).to_string();
                truncate_output(&mut stdout_text, request.max_output_bytes);
                truncate_output(&mut stderr_text, request.max_output_bytes);
                Ok(ShellResult {
                    stdout: stdout_text,
                    stderr: stderr_text,
                    exit_code: status.code().unwrap_or(-1),
                    success: status.success(),
                })
            }
            Ok(Err(error)) => Err(ShellError::Spawn(error.to_string())),
            Err(_elapsed) => {
                // Fail closed: drop partial output, kill then reap the child.
                let _ = child.kill().await;
                let _ = child.wait().await;
                Err(ShellError::TimedOut(timeout_ms))
            }
        }
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<Shell> {
    let shell = Shell::new();
    ctx.provide("shell", shell.clone()).await?;
    Ok(shell)
}
