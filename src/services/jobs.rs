//! Work on another thread whose one result a frame picks up (docs/client-refactor-next-steps.md,
//! S3). A `Job` is idle until work starts, busy while it runs, and idle again once its result
//! has been taken or its thread ended without one. Dropping or cancelling a job drops its
//! receiving end: the thread runs to its end and its result goes nowhere.

use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(crate) struct Job<T> {
    rx: Option<Receiver<T>>,
}

impl<T> Default for Job<T> {
    fn default() -> Self {
        Self::idle()
    }
}

impl<T: Send + 'static> Job<T> {
    /// Runs `work` on a thread of its own.
    pub(crate) fn spawn(work: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
        Self { rx: Some(rx) }
    }
}

impl<T> Job<T> {
    /// No work.
    pub(crate) const fn idle() -> Self {
        Self { rx: None }
    }

    /// Work started elsewhere that sends its one result to `rx`.
    pub(crate) fn from_receiver(rx: Receiver<T>) -> Self {
        Self { rx: Some(rx) }
    }

    /// Whether no work is running: none was started, or its result was taken, or it ended
    /// without one.
    pub(crate) fn is_idle(&self) -> bool {
        self.rx.is_none()
    }

    /// Whether work is running and its result hasn't been taken.
    pub(crate) fn is_busy(&self) -> bool {
        self.rx.is_some()
    }

    /// The result, once, when it has come; the job is idle after it.
    pub(crate) fn poll(&mut self) -> Option<T> {
        self.poll_ended().flatten()
    }

    /// Like `poll`, but also says when the work ended without a result (its thread panicked):
    /// `Some(None)` then. The job is idle after either.
    pub(crate) fn poll_ended(&mut self) -> Option<Option<T>> {
        let ended = match self.rx.as_ref()?.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => None,
        };
        self.rx = None;
        Some(ended)
    }

    /// Stops waiting for the work: its result, if it still comes, is dropped.
    pub(crate) fn cancel(&mut self) {
        self.rx = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Polls `job` until it has something to say, for at most a few seconds.
    fn wait<T>(job: &mut Job<T>) -> Option<Option<T>> {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if let Some(ended) = job.poll_ended() {
                return Some(ended);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        None
    }

    #[test]
    fn a_job_hands_its_result_over_once_and_is_idle_after() {
        let mut idle: Job<u32> = Job::default();
        assert!(idle.is_idle() && idle.poll().is_none());

        let mut job = Job::spawn(|| 7);
        assert!(job.is_busy());
        assert_eq!(wait(&mut job), Some(Some(7)), "the result never came");
        assert!(job.is_idle());
        assert_eq!(job.poll(), None, "a result is handed over once");
    }

    #[test]
    fn a_job_whose_thread_ends_without_a_result_says_so_and_is_idle() {
        let (tx, rx) = mpsc::channel::<u32>();
        let mut job = Job::from_receiver(rx);
        assert_eq!(job.poll_ended(), None, "nothing yet: still busy");
        assert!(job.is_busy());
        drop(tx);
        assert_eq!(job.poll_ended(), Some(None));
        assert!(job.is_idle());
    }

    #[test]
    fn a_cancelled_job_is_idle_and_its_late_result_goes_nowhere() {
        let (go, wait_for_go) = mpsc::channel::<()>();
        let mut job = Job::spawn(move || {
            let _ = wait_for_go.recv();
            1
        });
        job.cancel();
        assert!(job.is_idle());
        let _ = go.send(());
        assert_eq!(job.poll(), None);
    }
}
