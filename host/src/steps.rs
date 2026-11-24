//! Off-thread steps: run a task's slow work (the model call and the tool
//! calls it makes) on its own thread, so the pump thread keeps starting and
//! collecting other tasks.
//!
//! The pump hands a job to [`StepRunner::spawn`] and returns at once. A later
//! pump pass calls [`StepRunner::drain`] to pick up the finished results. At
//! most one step per task is in flight. The runner also remembers when each
//! step started, which the [`crate::watchdog`] reads to spot a step that has
//! run too long.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

pub struct StepRunner<T> {
    tx: Sender<(String, T)>,
    rx: Receiver<(String, T)>,
    started: HashMap<String, Instant>,
}

impl<T: Send + 'static> Default for StepRunner<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + 'static> StepRunner<T> {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            started: HashMap::new(),
        }
    }

    /// Start `job` for `task_id` on a new thread. Returns `false` and drops
    /// the job when that task already has a step in flight, or when the
    /// thread could not be started.
    pub fn spawn(&mut self, task_id: &str, job: impl FnOnce() -> T + Send + 'static) -> bool {
        self.spawn_at(task_id, Instant::now(), job)
    }

    /// [`Self::spawn`] with an explicit start time (tests).
    pub fn spawn_at(
        &mut self,
        task_id: &str,
        now: Instant,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> bool {
        if self.started.contains_key(task_id) {
            return false;
        }
        let tx = self.tx.clone();
        let id = task_id.to_string();
        let spawned = std::thread::Builder::new()
            .name(format!("robroz-step-{task_id}"))
            .spawn(move || {
                let out = job();
                // The receiver only goes away with the runner; nothing to do then.
                let _ = tx.send((id, out));
            });
        match spawned {
            Ok(_) => {
                self.started.insert(task_id.to_string(), now);
                true
            }
            Err(err) => {
                eprintln!("steps: spawn for {task_id}: {err}");
                false
            }
        }
    }

    /// Finished steps since the last call, oldest first. Never blocks.
    pub fn drain(&mut self) -> Vec<(String, T)> {
        let mut done = Vec::new();
        while let Ok((id, out)) = self.rx.try_recv() {
            self.started.remove(&id);
            done.push((id, out));
        }
        done
    }

    /// Whether `task_id` has a step in flight.
    pub fn is_running(&self, task_id: &str) -> bool {
        self.started.contains_key(task_id)
    }

    /// Steps in flight and when each started.
    pub fn in_flight(&self) -> impl Iterator<Item = (&str, Instant)> {
        self.started.iter().map(|(id, at)| (id.as_str(), *at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn drain_until<T: Send + 'static>(runner: &mut StepRunner<T>, n: usize) -> Vec<(String, T)> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut out = Vec::new();
        while out.len() < n && Instant::now() < deadline {
            out.extend(runner.drain());
            std::thread::sleep(Duration::from_millis(5));
        }
        out
    }

    #[test]
    fn spawn_returns_before_the_job_finishes() {
        let mut runner = StepRunner::new();
        let (release, gate) = mpsc::channel::<()>();
        assert!(runner.spawn("t1", move || {
            gate.recv().unwrap();
            42
        }));
        // The caller is free while the job waits.
        assert!(runner.is_running("t1"));
        assert!(runner.drain().is_empty());
        release.send(()).unwrap();
        let done = drain_until(&mut runner, 1);
        assert_eq!(done, vec![("t1".to_string(), 42)]);
        assert!(!runner.is_running("t1"));
    }

    #[test]
    fn one_step_per_task() {
        let mut runner = StepRunner::new();
        let (release, gate) = mpsc::channel::<()>();
        assert!(runner.spawn("t1", move || {
            gate.recv().unwrap();
            1
        }));
        assert!(
            !runner.spawn("t1", || 2),
            "second step for the same task is refused"
        );
        assert!(runner.spawn("t2", || 3), "another task is not held up");
        release.send(()).unwrap();
        let mut done = drain_until(&mut runner, 2);
        done.sort();
        assert_eq!(done, vec![("t1".to_string(), 1), ("t2".to_string(), 3)]);
    }

    #[test]
    fn in_flight_reports_start_time() {
        let mut runner = StepRunner::new();
        let at = Instant::now();
        let (release, gate) = mpsc::channel::<()>();
        runner.spawn_at("t1", at, move || gate.recv().unwrap());
        assert_eq!(runner.in_flight().collect::<Vec<_>>(), vec![("t1", at)]);
        release.send(()).unwrap();
        drain_until(&mut runner, 1);
        assert_eq!(runner.in_flight().count(), 0);
    }
}
