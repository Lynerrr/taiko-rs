//! A few threads for the pictures the screens ask for (song select's card strips and banner, a
//! map's own pictures, the download tab's covers), so reading them never takes more of the
//! machine than it can spare: one thread on a computer of one or two cores, two on the rest.
//! Each read used to get a thread of its own, and flying through the list started one for every
//! set passing by, all decoding at once next to the game on a two-core CPU.
//!
//! A piece of work can come with a `Ticket`: called off (the card scrolled away, the banner
//! isn't the selection's any more), the work is skipped when its turn comes instead of being
//! read for nothing.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, Once};

type Task = Box<dyn FnOnce() + Send>;

/// How many threads read pictures on a machine of `cores`: the rest is the game's.
pub(crate) fn threads_for(cores: usize) -> usize {
    if cores <= 2 { 1 } else { 2 }
}

struct Pool {
    queue: Mutex<VecDeque<Task>>,
    ready: Condvar,
}

static POOL: LazyLock<Pool> = LazyLock::new(|| Pool {
    queue: Mutex::new(VecDeque::new()),
    ready: Condvar::new(),
});
static STARTED: Once = Once::new();

/// Runs `task` on one of the picture threads, after the work given before it.
pub(crate) fn run(task: impl FnOnce() + Send + 'static) {
    STARTED.call_once(|| {
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        for i in 0..threads_for(cores) {
            let spawned = std::thread::Builder::new()
                .name(format!("pictures {i}"))
                .spawn(work);
            if let Err(e) = spawned {
                eprintln!("picture thread: {e}");
            }
        }
    });
    let mut queue = POOL.queue.lock().unwrap_or_else(|e| e.into_inner());
    queue.push_back(Box::new(task));
    drop(queue);
    POOL.ready.notify_one();
}

fn work() {
    // Below the game's threads: what's on screen is drawn first, pictures come in after.
    set_priority(Priority::BelowNormal);
    loop {
        let task = {
            let mut queue = POOL.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(task) = queue.pop_front() {
                    break task;
                }
                queue = POOL.ready.wait(queue).unwrap_or_else(|e| e.into_inner());
            }
        };
        // A picture that can't be read mustn't take the thread with it.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
    }
}

/// Whether a piece of work is still wanted: `cancel` it and the thread skips it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Ticket(Arc<AtomicBool>);

impl Ticket {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub(crate) fn wanted(&self) -> bool {
        !self.0.load(Ordering::Relaxed)
    }
}

/// How much of the CPU a thread takes before others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Priority {
    /// Work for what's on screen (the pictures being read): after the game's threads.
    BelowNormal,
    /// Work made ahead (the library's picture copies): only what's left over.
    Lowest,
}

/// Puts the calling thread at `priority` (Linux: its nice value; Windows: the thread's
/// priority). A failure leaves it as it was.
pub(crate) fn set_priority(priority: Priority) {
    #[cfg(target_os = "linux")]
    {
        let nice = match priority {
            Priority::BelowNormal => 10,
            Priority::Lowest => 19,
        };
        // SAFETY: plain syscalls on this thread's own id; a failure only leaves the priority.
        unsafe {
            let tid = libc::gettid();
            libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, nice);
        }
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentThread() -> *mut std::ffi::c_void;
            fn SetThreadPriority(thread: *mut std::ffi::c_void, priority: i32) -> i32;
        }
        const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;
        const THREAD_PRIORITY_LOWEST: i32 = -2;
        let level = match priority {
            Priority::BelowNormal => THREAD_PRIORITY_BELOW_NORMAL,
            Priority::Lowest => THREAD_PRIORITY_LOWEST,
        };
        // SAFETY: GetCurrentThread's pseudo-handle is always valid for the calling thread and
        // needs no closing; a failed call only leaves the priority as it was.
        unsafe {
            SetThreadPriority(GetCurrentThread(), level);
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = priority;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn a_small_machine_reads_on_one_thread_a_bigger_one_on_two() {
        assert_eq!(threads_for(1), 1);
        assert_eq!(
            threads_for(2),
            1,
            "a two-core CPU keeps a core for the game"
        );
        assert_eq!(threads_for(4), 2);
        assert_eq!(
            threads_for(16),
            2,
            "never more than two, however many there are"
        );
    }

    #[test]
    fn work_runs_on_the_picture_threads_and_a_called_off_piece_is_skipped() {
        let (tx, rx) = mpsc::channel();
        let ticket = Ticket::default();
        // Held up behind work that waits, it's called off before its turn.
        let (go, wait) = mpsc::channel::<()>();
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        let gate = std::sync::Arc::new(std::sync::Mutex::new(wait));
        for _ in 0..threads_for(cores) {
            let gate = gate.clone();
            run(move || {
                let _ = gate.lock().unwrap().recv_timeout(Duration::from_secs(5));
            });
        }
        let (t, skipped) = (ticket.clone(), tx.clone());
        run(move || {
            let _ = skipped.send(if t.wanted() { "read" } else { "skipped" });
        });
        ticket.cancel();
        run(move || {
            let _ = tx.send("next");
        });
        for _ in 0..threads_for(cores) {
            let _ = go.send(());
        }
        let got: Vec<&str> = (0..2)
            .map(|_| rx.recv_timeout(Duration::from_secs(10)).unwrap())
            .collect();
        assert!(
            got.contains(&"skipped"),
            "called off before its turn: {got:?}"
        );
        assert!(
            got.contains(&"next"),
            "the work after it still runs: {got:?}"
        );
    }

    #[test]
    fn a_thread_can_be_put_below_the_game() {
        // On its own thread, so the test runner's threads keep their priority.
        std::thread::spawn(|| {
            set_priority(Priority::BelowNormal);
            set_priority(Priority::Lowest);
        })
        .join()
        .unwrap();
    }
}
