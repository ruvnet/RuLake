//! Bounded worker pool — the ADR-004 §6 commitment.
//!
//! The MCP-side tokio task forwards each `RuLake::*` call to a
//! dedicated `rayon::ThreadPool` of size `cores * 2` (operator-overridable),
//! through a bounded `flume` channel of capacity `max_inflight`.
//! Submission past the cap returns the `Degraded` error immediately —
//! never unbounded queueing.
//!
//! This isolates RuLake CPU work from the tokio reactor that owns the
//! wire (so a 50 ms scan can't starve a heartbeat) and bounds the
//! worst-case scan-thread count regardless of MCP-call burstiness
//! (so one hot collection can't spawn unbounded scan threads).

use std::sync::Arc;

use rayon::ThreadPoolBuilder;

/// Why a `WorkerPool::submit` failed. `Degraded` is the ADR-004 §6
/// backpressure signal — never an unbounded queue.
#[derive(Debug)]
pub enum SubmitError {
    Degraded { inflight: usize, cap: usize },
    ShutDown,
    WorkerPanic,
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Degraded { inflight, cap } => {
                write!(f, "worker pool degraded: at-capacity ({inflight} / {cap})")
            }
            Self::ShutDown => write!(f, "worker pool shut down"),
            Self::WorkerPanic => write!(f, "worker panicked"),
        }
    }
}
impl std::error::Error for SubmitError {}

#[derive(Clone)]
pub struct WorkerPool {
    inner: Arc<Inner>,
}

struct Inner {
    pool: rayon::ThreadPool,
    inflight: std::sync::atomic::AtomicUsize,
    max_inflight: usize,
}

impl WorkerPool {
    pub fn new(workers: usize, max_inflight: usize) -> anyhow::Result<Self> {
        let workers = if workers == 0 {
            num_cores() * 2
        } else {
            workers.min(256)
        };
        let pool = ThreadPoolBuilder::new()
            .num_threads(workers)
            .thread_name(|i| format!("rulake-worker-{i}"))
            .build()
            .map_err(|e| anyhow::anyhow!("rayon pool build: {e}"))?;
        Ok(Self {
            inner: Arc::new(Inner {
                pool,
                inflight: std::sync::atomic::AtomicUsize::new(0),
                max_inflight,
            }),
        })
    }

    /// Submit `f` to the pool. Returns `Degraded` immediately if
    /// `max_inflight` would be exceeded — the backpressure path from
    /// ADR-004 §6 (caller turns this into a `degraded` `_meta` block).
    pub async fn submit<T, F>(&self, f: F) -> Result<T, SubmitError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        use std::sync::atomic::Ordering;

        let inner = Arc::clone(&self.inner);
        let prev = inner.inflight.fetch_add(1, Ordering::SeqCst);
        if prev >= inner.max_inflight {
            inner.inflight.fetch_sub(1, Ordering::SeqCst);
            return Err(SubmitError::Degraded {
                inflight: prev,
                cap: inner.max_inflight,
            });
        }

        let (tx, rx) = flume::bounded::<T>(1);
        let inner_for_spawn = Arc::clone(&inner); // borrow `pool` then move the second clone
        inner.pool.spawn(move || {
            let out = f();
            // Send may fail if the receiver was dropped (caller
            // cancelled); that's fine — drop the result on the floor.
            let _ = tx.send(out);
            inner_for_spawn.inflight.fetch_sub(1, Ordering::SeqCst);
        });

        rx.recv_async().await.map_err(|_| SubmitError::WorkerPanic)
    }

    pub fn inflight(&self) -> usize {
        self.inner
            .inflight
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn max_inflight(&self) -> usize {
        self.inner.max_inflight
    }
}

fn num_cores() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(4)
}
