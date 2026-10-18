//! The threads an open journal runs on.
//!
//! Each [`super::LedgelineApp`] owns a small tokio runtime: two workers (the
//! router's handlers are short; anything heavy already goes to the blocking
//! pool through the server's `compute`) and at most four blocking threads
//! (journal I/O, report computation). Every thread gets the dataless-file
//! policy ([`crate::iopolicy`]) as it starts, so no Ledgeline thread waits on a
//! cloud download.
//!
//! Its own runtime rather than the caller's because the caller has none: UniFFI
//! drives exported `async fn`s on its own executor, from Swift, and the router
//! needs tokio. Work is spawned onto this runtime and the `JoinHandle` awaited,
//! which any executor can do.

use std::io;

use crate::iopolicy;

/// Tokio worker threads per open journal.
const WORKER_THREADS: usize = 2;

/// Upper bound on each journal's blocking pool.
const MAX_BLOCKING_THREADS: usize = 4;

/// The runtime for one open journal.
pub(crate) fn build() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKER_THREADS)
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .thread_name("ledgeline-mobile")
        .on_thread_start(iopolicy::apply_to_current_thread)
        .enable_all()
        .build()
}

/// A tokio runtime that shuts down without waiting.
///
/// Dropping a plain `Runtime` blocks until every `spawn_blocking` task ends.
/// The last reference to a `LedgelineApp` is dropped on a Swift thread, often
/// the main one, which must not block.
pub(crate) struct BackgroundRuntime(Option<tokio::runtime::Runtime>);

impl BackgroundRuntime {
    pub(crate) fn new(runtime: tokio::runtime::Runtime) -> Self {
        Self(Some(runtime))
    }

    /// The runtime. Present for this value's whole life; `None` only inside
    /// `drop`.
    fn runtime(&self) -> &tokio::runtime::Runtime {
        match &self.0 {
            Some(runtime) => runtime,
            None => unreachable!("the runtime is taken only in drop"),
        }
    }

    pub(crate) fn handle(&self) -> &tokio::runtime::Handle {
        self.runtime().handle()
    }
}

impl Drop for BackgroundRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}

#[cfg(all(test, target_vendor = "apple"))]
mod tests {
    #[test]
    fn runtime_threads_carry_their_own_dataless_policy() {
        let runtime = super::build().unwrap();
        let blocking = runtime
            .block_on(async {
                tokio::task::spawn_blocking(crate::iopolicy::current_thread_materializes).await
            })
            .unwrap();
        // Another test may flip the global toggle concurrently; what matters
        // is that the thread carries a policy of its own.
        assert!(blocking.is_some(), "blocking threads get the policy");
        let worker = runtime
            .block_on(async {
                tokio::spawn(async { crate::iopolicy::current_thread_materializes() }).await
            })
            .unwrap();
        assert!(worker.is_some(), "workers too");
    }
}
