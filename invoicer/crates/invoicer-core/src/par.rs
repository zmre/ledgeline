//! Parallel map on scoped std threads (§5.6 "parallel open"). rayon isn't
//! in the workspace's lockfile, and a scan is one embarrassingly parallel
//! map, so this is all it needs.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Below this many items a map runs on the calling thread: spawning costs
/// more than it saves.
const SEQUENTIAL_BELOW: usize = 64;

/// `items.iter().map(f)`, split over the available cores, in order.
pub(crate) fn par_map<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    if threads <= 1 || items.len() < SEQUENTIAL_BELOW {
        return items.iter().map(f).collect();
    }
    // Several chunks per worker so one slow file doesn't idle the rest; at
    // most `threads` workers, each pulling the next chunk until none is left.
    let chunk = items.len().div_ceil(threads * 4).max(1);
    let chunks: Vec<&[T]> = items.chunks(chunk).collect();
    let next = AtomicUsize::new(0);
    let worker = || {
        std::iter::from_fn(|| {
            let at = next.fetch_add(1, Ordering::Relaxed);
            chunks
                .get(at)
                .map(|part| (at, part.iter().map(&f).collect::<Vec<R>>()))
        })
        .collect::<Vec<_>>()
    };
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads.min(chunks.len()))
            .map(|_| scope.spawn(worker))
            .collect();
        let mut done: Vec<(usize, Vec<R>)> = handles
            .into_iter()
            .flat_map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect();
        done.sort_unstable_by_key(|(at, _)| *at);
        done.into_iter().flat_map(|(_, part)| part).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_at_every_size() {
        for n in [0, 1, 63, 64, 1_000, 5_001] {
            let items: Vec<usize> = (0..n).collect();
            assert_eq!(
                par_map(&items, |i| i * 2),
                items.iter().map(|i| i * 2).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn runs_on_at_most_one_thread_per_core() {
        let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        let items: Vec<usize> = (0..5_000).collect();
        let used: std::collections::HashSet<_> = par_map(&items, |_| std::thread::current().id())
            .into_iter()
            .collect();
        assert!(
            used.len() <= threads,
            "{} threads for {threads} cores",
            used.len()
        );
    }
}
