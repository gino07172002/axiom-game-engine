//! Bounded background jobs for the editor/game main thread: cooperative cancellation,
//! countable progress (done / total), a watchdog deadline and a timed join on shutdown.
//! A worker that ignores cancellation is detached after a grace period, never awaited
//! forever (docs/lessons-editor-stalls.zh-TW.md).
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        mpsc, Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Shared between the main thread and one worker.
#[derive(Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
    cancel: AtomicBool,
}
impl Progress {
    pub fn set_total(&self, total: u64) {
        self.total.store(total, Relaxed);
    }
    pub fn set_done(&self, done: u64) {
        self.done.store(done, Relaxed);
    }
    pub fn done(&self) -> u64 {
        self.done.load(Relaxed)
    }
    pub fn total(&self) -> u64 {
        self.total.load(Relaxed)
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }
}

/// Why a job stopped before finishing on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Cancelled,
    TimedOut,
}
impl Stop {
    pub fn name(self) -> &'static str {
        match self {
            Stop::Cancelled => "cancelled",
            Stop::TimedOut => "timed_out",
        }
    }
}

/// How a job ended, as seen by the main thread.
pub enum Outcome<T> {
    /// The worker returned. `stop` says whether cancellation or the watchdog asked it to.
    Done { value: T, stop: Option<Stop> },
    /// The worker did not answer a stop request within the grace period and was detached.
    Abandoned { stop: Stop },
}

/// After a stop request, how long `poll` waits for the worker before detaching it.
pub const STOP_GRACE: Duration = Duration::from_secs(5);

pub struct Job<T> {
    pub kind: &'static str,
    pub label: String,
    started: Instant,
    timeout: Duration,
    progress: Arc<Progress>,
    result: mpsc::Receiver<T>,
    handle: Option<JoinHandle<()>>,
    stop: Option<(Stop, Instant)>,
}

impl<T: Send + 'static> Job<T> {
    /// Starts `work` on a named worker thread. `work` must check `Progress::cancelled`
    /// regularly and should report `set_total` / `set_done`.
    pub fn spawn(
        kind: &'static str,
        label: impl Into<String>,
        timeout: Duration,
        work: impl FnOnce(&Progress) -> T + Send + 'static,
    ) -> anyhow::Result<Self> {
        let progress = Arc::new(Progress::default());
        let shared = progress.clone();
        let (sender, result) = mpsc::sync_channel(1);
        let handle = std::thread::Builder::new()
            .name(format!("axiom-job-{kind}"))
            .spawn(move || {
                let _ = sender.send(work(&shared));
            })?;
        Ok(Self {
            kind,
            label: label.into(),
            started: Instant::now(),
            timeout,
            progress,
            result,
            handle: Some(handle),
            stop: None,
        })
    }
}

impl<T> Job<T> {
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Non-blocking; call once per frame. Enforces the watchdog deadline.
    pub fn poll(&mut self) -> Option<Outcome<T>> {
        self.poll_within(Duration::ZERO)
    }

    /// `poll` that waits at most `wait` for the result (bounded by the caller's frame
    /// budget; never an unbounded wait on the main thread).
    pub fn poll_within(&mut self, wait: Duration) -> Option<Outcome<T>> {
        let received = if wait.is_zero() {
            self.result.try_recv()
        } else {
            self.result.recv_timeout(wait).map_err(|e| match e {
                mpsc::RecvTimeoutError::Timeout => mpsc::TryRecvError::Empty,
                mpsc::RecvTimeoutError::Disconnected => mpsc::TryRecvError::Disconnected,
            })
        };
        match received {
            Ok(value) => {
                if let Some(handle) = self.handle.take() {
                    // The worker has sent its result and is returning.
                    let _ = handle.join();
                }
                return Some(Outcome::Done {
                    value,
                    stop: self.stop.map(|(s, _)| s),
                });
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                // The worker panicked before sending.
                self.handle.take();
                return Some(Outcome::Abandoned {
                    stop: self.stop.map_or(Stop::Cancelled, |(s, _)| s),
                });
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if self.stop.is_none() && self.started.elapsed() >= self.timeout {
            self.request_stop(Stop::TimedOut);
        }
        if let Some((stop, at)) = self.stop {
            if at.elapsed() >= STOP_GRACE {
                // Never block the main thread on a worker stuck inside one step.
                self.handle.take();
                return Some(Outcome::Abandoned { stop });
            }
        }
        None
    }

    pub fn cancel(&mut self) {
        self.request_stop(Stop::Cancelled);
    }

    fn request_stop(&mut self, stop: Stop) {
        if self.stop.is_none() {
            self.stop = Some((stop, Instant::now()));
        }
        self.progress.cancel();
    }

    pub fn observation(&self) -> Value {
        json!({
            "kind":self.kind,
            "label":self.label,
            "state":self.stop.map_or("running", |(s, _)| if s == Stop::Cancelled {"cancelling"} else {"timing_out"}),
            "done":self.progress.done(),
            "total":self.progress.total(),
            "elapsed_ms":self.started.elapsed().as_millis() as u64,
            "timeout_ms":self.timeout.as_millis() as u64,
        })
    }

    /// Stops caring about the result without waiting: the worker finishes on its own and
    /// its result is dropped. For work that cannot be cancelled mid-way (e.g. a runtime
    /// build) and is superseded.
    pub fn detach(mut self) {
        self.request_stop(Stop::Cancelled);
        self.handle.take();
    }

    pub fn progress(&self) -> (u64, u64) {
        (self.progress.done(), self.progress.total())
    }

    /// Cancels and waits at most `limit` for the worker. Returns whether it was joined;
    /// otherwise the thread is detached.
    pub fn shutdown(&mut self, limit: Duration) -> bool {
        self.request_stop(Stop::Cancelled);
        let Some(handle) = self.handle.take() else {
            return true;
        };
        let deadline = Instant::now() + limit;
        while !handle.is_finished() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = handle.join();
        true
    }
}

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        if self.handle.is_some() && !self.shutdown(Duration::from_secs(2)) {
            eprintln!(
                "job {} '{}' did not stop within 2 s; detached",
                self.kind, self.label
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait<T>(job: &mut Job<T>) -> Outcome<T> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(outcome) = job.poll() {
                return outcome;
            }
            assert!(Instant::now() < deadline, "job never finished");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn cancel_stops_a_cooperative_worker_and_delivers_its_result() {
        let mut job = Job::spawn("test", "spin", Duration::from_secs(60), |p| {
            p.set_total(u64::MAX);
            let mut n = 0;
            while !p.cancelled() {
                n += 1;
                p.set_done(n);
            }
            n
        })
        .unwrap();
        while job.progress().0 == 0 {
            std::thread::yield_now();
        }
        assert!(job.poll().is_none());
        job.cancel();
        match wait(&mut job) {
            Outcome::Done { value, stop } => {
                assert!(value > 0);
                assert_eq!(stop, Some(Stop::Cancelled));
            }
            Outcome::Abandoned { .. } => panic!("cooperative worker was abandoned"),
        }
    }

    #[test]
    fn watchdog_times_out_a_long_worker() {
        let mut job = Job::spawn("test", "slow", Duration::from_millis(20), |p| {
            while !p.cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            "stopped"
        })
        .unwrap();
        match wait(&mut job) {
            Outcome::Done { value, stop } => {
                assert_eq!(value, "stopped");
                assert_eq!(stop, Some(Stop::TimedOut));
            }
            Outcome::Abandoned { .. } => panic!("cooperative worker was abandoned"),
        }
    }

    #[test]
    fn shutdown_does_not_wait_forever_for_an_uncooperative_worker() {
        let (release, gate) = mpsc::channel::<()>();
        let mut job = Job::spawn("test", "stuck", Duration::from_secs(60), move |_| {
            let _ = gate.recv_timeout(Duration::from_secs(30));
        })
        .unwrap();
        let started = Instant::now();
        assert!(!job.shutdown(Duration::from_millis(50)));
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(release);
    }
}
