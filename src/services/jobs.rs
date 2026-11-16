//! `ctx.jobs`: process-local background job registry.
//!
//! Upstream: `packages/jobs/jobs/src/index.ts` and `jobs-local/src/index.ts`
//! in deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! This port keeps lifecycle state and a bounded output ring in memory;
//! producers supply their own execution resources.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Stopping,
    Succeeded,
    Failed,
    Killed,
}

impl JobStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Killed)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobView {
    pub id: u64,
    pub owner: Option<String>,
    pub status: JobStatus,
    pub output_len: usize,
}

struct OutputRing {
    bytes: Mutex<Vec<u8>>,
    retain_bytes: usize,
}

impl OutputRing {
    fn new(retain_bytes: usize) -> Self {
        Self {
            bytes: Mutex::new(Vec::new()),
            retain_bytes,
        }
    }

    fn append(&self, chunk: &[u8]) {
        let mut guard = self.bytes.lock().expect("job output poisoned");
        guard.extend_from_slice(chunk);
        if guard.len() > self.retain_bytes {
            let drop_count = guard.len() - self.retain_bytes;
            guard.drain(..drop_count);
        }
    }

    fn read(&self, offset: usize) -> (String, usize) {
        let guard = self.bytes.lock().expect("job output poisoned");
        let start = offset.min(guard.len());
        let text = String::from_utf8_lossy(&guard[start..]).to_string();
        (text, guard.len())
    }
}

struct JobRecord {
    id: u64,
    owner: Option<String>,
    status: JobStatus,
    ring: OutputRing,
    terminal: Arc<(Mutex<bool>, Condvar)>,
}

#[derive(Clone)]
pub struct JobHandle {
    record: Arc<RwLock<JobRecord>>,
}

impl JobHandle {
    pub fn id(&self) -> u64 {
        self.record.read().expect("job record poisoned").id
    }

    pub fn status(&self) -> JobStatus {
        self.record.read().expect("job record poisoned").status
    }

    pub fn append(&self, chunk: impl AsRef<[u8]>) {
        let guard = self.record.read().expect("job record poisoned");
        guard.ring.append(chunk.as_ref());
    }

    /// Read output from an absolute byte offset; returns text and the new end.
    pub fn read(&self, offset: usize) -> (String, usize) {
        let guard = self.record.read().expect("job record poisoned");
        guard.ring.read(offset)
    }

    pub fn settle(&self, status: JobStatus) {
        {
            let mut guard = self.record.write().expect("job record poisoned");
            if !guard.status.is_terminal() {
                guard.status = status;
            }
        }
        let terminal = &self.record.read().expect("job record poisoned").terminal;
        let (lock, cvar) = &**terminal;
        *lock.lock().expect("job terminal poisoned") = true;
        cvar.notify_all();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobError {
    NotFound(u64),
    NotOwner { id: u64, owner: String },
    AlreadyTerminal(u64),
    MaxConcurrent { owner: Option<String>, max: usize },
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "job {id} not found"),
            Self::NotOwner { id, owner } => write!(f, "job {id} is not owned by {owner}"),
            Self::AlreadyTerminal(id) => write!(f, "job {id} already settled"),
            Self::MaxConcurrent { owner, max } => {
                write!(f, "owner {owner:?} already has {max} active jobs")
            }
        }
    }
}

impl std::error::Error for JobError {}

#[derive(Default)]
struct RegistryState {
    next_id: AtomicU64,
    jobs: HashMap<u64, JobHandle>,
    max_concurrent: usize,
    retain_bytes: usize,
}

/// The shared background job registry.
#[derive(Default, Clone)]
pub struct JobRegistry {
    state: Arc<RwLock<RegistryState>>,
}

impl JobRegistry {
    pub fn new(max_concurrent: usize, retain_bytes: usize) -> Self {
        Self {
            state: Arc::new(RwLock::new(RegistryState {
                next_id: AtomicU64::new(1),
                jobs: HashMap::new(),
                max_concurrent: max_concurrent.max(1),
                retain_bytes: retain_bytes.max(1024),
            })),
        }
    }

    /// Register a new running job. The returned handle is the producer's
    /// resource; observers use the id through this registry.
    pub fn start(&self, owner: Option<String>) -> std::result::Result<JobHandle, JobError> {
        let mut guard = self.state.write().expect("job registry poisoned");
        if guard.max_concurrent > 0 {
            let active = guard
                .jobs
                .values()
                .filter(|handle| {
                    let status = handle.status();
                    !status.is_terminal()
                        && handle.record.read().expect("job record poisoned").owner == owner
                })
                .count();
            if active >= guard.max_concurrent {
                return Err(JobError::MaxConcurrent {
                    owner,
                    max: guard.max_concurrent,
                });
            }
        }
        let id = guard.next_id.fetch_add(1, Ordering::SeqCst);
        let retain = guard.retain_bytes;
        let handle = JobHandle {
            record: Arc::new(RwLock::new(JobRecord {
                id,
                owner: owner.clone(),
                status: JobStatus::Running,
                ring: OutputRing::new(retain),
                terminal: Arc::new((Mutex::new(false), Condvar::new())),
            })),
        };
        guard.jobs.insert(id, handle.clone());
        Ok(handle)
    }

    pub fn get(&self, id: u64) -> Option<JobHandle> {
        self.state
            .read()
            .expect("job registry poisoned")
            .jobs
            .get(&id)
            .cloned()
    }

    fn authorize(&self, id: u64, caller: Option<&str>) -> std::result::Result<JobHandle, JobError> {
        let handle = self.get(id).ok_or(JobError::NotFound(id))?;
        let job_owner = handle
            .record
            .read()
            .expect("job record poisoned")
            .owner
            .clone();
        if let (Some(job_owner), Some(caller)) = (job_owner, caller)
            && job_owner != caller
        {
            return Err(JobError::NotOwner {
                id,
                owner: caller.to_string(),
            });
        }
        Ok(handle)
    }

    pub fn view(&self, id: u64, caller: Option<&str>) -> std::result::Result<JobView, JobError> {
        let handle = self.authorize(id, caller)?;
        let record = handle.record.read().expect("job record poisoned");
        Ok(JobView {
            id: record.id,
            owner: record.owner.clone(),
            status: record.status,
            output_len: record.ring.read(0).1,
        })
    }

    pub fn list(&self, caller: Option<&str>) -> Vec<JobView> {
        let handles: Vec<JobHandle> = self
            .state
            .read()
            .expect("job registry poisoned")
            .jobs
            .values()
            .cloned()
            .collect();
        let mut views = Vec::new();
        for handle in handles {
            let record = handle.record.read().expect("job record poisoned");
            if let (Some(owner), Some(caller)) = (&record.owner, caller)
                && owner != caller
            {
                continue;
            }
            views.push(JobView {
                id: record.id,
                owner: record.owner.clone(),
                status: record.status,
                output_len: record.ring.read(0).1,
            });
        }
        views.sort_by_key(|view| view.id);
        views
    }

    pub fn read(
        &self,
        id: u64,
        offset: usize,
        caller: Option<&str>,
    ) -> std::result::Result<(String, usize), JobError> {
        Ok(self.authorize(id, caller)?.read(offset))
    }

    pub fn kill(&self, id: u64, caller: Option<&str>) -> std::result::Result<(), JobError> {
        let handle = self.authorize(id, caller)?;
        if handle.status().is_terminal() {
            return Err(JobError::AlreadyTerminal(id));
        }
        handle.settle(JobStatus::Killed);
        Ok(())
    }

    /// Wait for terminal state. Returns `true` when settled, `false` on
    /// timeout (milliseconds).
    pub fn wait(
        &self,
        id: u64,
        caller: Option<&str>,
        timeout_ms: u64,
    ) -> std::result::Result<bool, JobError> {
        let handle = self.authorize(id, caller)?;
        let terminal = {
            let record = handle.record.read().expect("job record poisoned");
            record.terminal.clone()
        };
        let (lock, cvar) = &*terminal;
        let settled = lock.lock().expect("job terminal poisoned");
        if *settled {
            return Ok(true);
        }
        let (guard, timeout_result) = cvar
            .wait_timeout_while(
                settled,
                std::time::Duration::from_millis(timeout_ms),
                |done| !*done,
            )
            .expect("job terminal poisoned");
        Ok(*guard && !timeout_result.timed_out())
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<JobRegistry> {
    let jobs = JobRegistry::new(10, 256 * 1024);
    ctx.provide("jobs", jobs.clone()).await?;
    Ok(jobs)
}
