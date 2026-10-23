//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};

use log::*;

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::execution_runner";

/// Runs a block's speculative work on up to `num_threads` threads, the calling thread among them.
///
/// The workers are plain scoped threads. Compiling a template runs a rayon `par_iter` while holding the template's
/// cache lock. A thread that is itself a rayon worker takes other queued work while it waits, which may be a unit that
/// wants the same template and then blocks on that lock. A plain thread hands the `par_iter` to rayon and only waits.
#[derive(Debug, Clone)]
pub struct ExecutionRunner {
    num_threads: usize,
}

impl ExecutionRunner {
    pub fn new(num_threads: usize) -> Self {
        Self {
            num_threads: num_threads.max(1),
        }
    }

    pub fn num_threads(&self) -> usize {
        self.num_threads
    }

    /// Applies `f` to every item, returning the results in item order. Workers take items in order, so the first item
    /// starts first. With one thread, or one item, everything runs on the calling thread.
    pub fn map<I, T, F>(&self, items: &[I], f: F) -> Vec<T>
    where
        I: Sync,
        T: Send,
        F: Fn(&I) -> T + Sync,
    {
        let num_workers = self.num_threads.min(items.len());
        if num_workers <= 1 {
            return items.iter().map(f).collect();
        }

        let next = AtomicUsize::new(0);
        let work = || {
            let mut done = Vec::new();
            loop {
                let idx = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(idx) else {
                    return done;
                };
                done.push((idx, f(item)));
            }
        };

        let mut results = items.iter().map(|_| None).collect::<Vec<_>>();
        thread::scope(|scope| {
            let workers = (1..num_workers)
                .filter_map(|idx| {
                    thread::Builder::new()
                        .name(format!("ootle-exec-{idx}"))
                        .spawn_scoped(scope, work)
                        .inspect_err(|err| {
                            warn!(target: LOG_TARGET, "⚠️ Could not start an execution thread: {err}");
                        })
                        .ok()
                })
                .collect::<Vec<_>>();
            // The calling thread works too, so every item is taken even if no thread could start
            for (idx, result) in work() {
                results[idx] = Some(result);
            }
            for worker in workers {
                let done = worker.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                for (idx, result) in done {
                    results[idx] = Some(result);
                }
            }
        });
        results
            .into_iter()
            .map(|result| result.expect("every item is taken by a worker"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Mutex, time::Duration};

    use super::*;

    #[test]
    fn results_keep_item_order() {
        let runner = ExecutionRunner::new(4);
        let results = runner.map(&(0..100).collect::<Vec<u32>>(), |n| n * 2);
        assert_eq!(results, (0..100).map(|n| n * 2).collect::<Vec<_>>());
    }

    #[test]
    fn items_run_on_several_threads() {
        let runner = ExecutionRunner::new(4);
        let threads = Mutex::new(HashSet::new());
        runner.map(&[(); 8], |_| {
            threads.lock().unwrap().insert(thread::current().id());
            thread::sleep(Duration::from_millis(20));
        });
        assert!(threads.into_inner().unwrap().len() > 1);
    }

    #[test]
    fn one_thread_runs_inline() {
        let runner = ExecutionRunner::new(1);
        let caller = thread::current().id();
        let threads = runner.map(&[(); 3], |_| thread::current().id());
        assert!(threads.iter().all(|id| *id == caller));
    }
}
