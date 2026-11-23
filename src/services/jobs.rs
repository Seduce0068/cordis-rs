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

type SettleCallback = dyn Fn(u64, JobStatus) + Send + Sync;

struct JobRecord {
    id: u64,
    owner: Option<String>,
    status: JobStatus,
    ring: OutputRing,
    terminal: Arc<(Mutex<bool>, Condvar)>,
    /// Notifies the registry when this job settles (swarm review R2-FG05).
    on_settle: Option<Arc<SettleCallback>>,
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

    /// Read output from an absolute byte offset; returns text and the current
    /// end. An offset beyond the retained bytes is clamped, so it yields an
    /// empty string together with the current end (swarm review R14-ST04).
    pub fn read(&self, offset: usize) -> (String, usize) {
        let guard = self.record.read().expect("job record poisoned");
        guard.ring.read(offset)
    }

    /// Settle the job and notify waiters. Lock order contract (swarm review
    /// R11-LC03): the record write guard is dropped before taking the terminal
    /// lock and before invoking `on_settle` (which takes the registry write
    /// lock). `JobRegistry::start` takes the registry lock before reading a
    /// record, so no path holds both in opposing order.
    pub fn settle(&self, status: JobStatus) {
        let callback = {
            let mut guard = self.record.write().expect("job record poisoned");
            if guard.status.is_terminal() {
                return;
            }
            guard.status = status;
            guard.on_settle.clone()
        };
        let terminal = &self.record.read().expect("job record poisoned").terminal;
        let (lock, cvar) = &**terminal;
        *lock.lock().expect("job terminal poisoned") = true;
        cvar.notify_all();
        if let Some(callback) = callback {
            callback(self.id(), status);
        }
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
    /// Unacknowledged completion notices and the cursor into them.
    notices: Vec<(u64, JobStatus)>,
    cursor: usize,
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
                notices: Vec::new(),
                cursor: 0,
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
        let weak_state = Arc::downgrade(&self.state);
        let handle = JobHandle {
            record: Arc::new(RwLock::new(JobRecord {
                id,
                owner: owner.clone(),
                status: JobStatus::Running,
                ring: OutputRing::new(retain),
                terminal: Arc::new((Mutex::new(false), Condvar::new())),
                on_settle: Some(Arc::new(move |settled_id, status| {
                    if let Some(state) = weak_state.upgrade() {
                        state
                            .write()
                            .expect("job registry poisoned")
                            .notices
                            .push((settled_id, status));
                    }
                })),
            })),
        };
        guard.jobs.insert(id, handle.clone());
        Ok(handle)
    }

    /// Completion notices newer than the acknowledged cursor, without
    /// advancing it (swarm review R2-FG05).
    pub fn pending_notices(&self) -> Vec<(u64, JobStatus)> {
        let guard = self.state.read().expect("job registry poisoned");
        guard.notices.iter().skip(guard.cursor).cloned().collect()
    }

    /// Advance the notice cursor after a notice has been collected. Acknowledged
    /// notices are drained and the cursor resets to zero: the index base moves,
    /// so the logical set of unread notices still shrinks monotonically
    /// (swarm review R11-RS01).
    pub fn acknowledge(&self, through: usize) {
        let mut guard = self.state.write().expect("job registry poisoned");
        let removed = through.min(guard.notices.len());
        guard.notices.drain(..removed);
        guard.cursor = 0;
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
        // Clamp here as well as in tool_jobs: a direct caller must not be able
        // to create an effectively unbounded wait (swarm review R17-PN10).
        let timeout_ms = timeout_ms.min(crate::ports::tool_jobs::MAX_WAIT_TIMEOUT_MS);
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
