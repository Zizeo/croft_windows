//! Cancelling a run: the process-tree killer of the runner the worker is blocked on, and
//! killing it.

use super::*;

impl RunControl {
    /// Cancel every request numbered up to `upto`: kill the running child's
    /// whole process tree (the runner, pytest-xdist workers, a `cargo
    /// test` binary), and keep any further child of those requests from
    /// starting.
    pub(super) fn cancel(&self, upto: u64) {
        self.cancelled_upto.fetch_max(upto, Ordering::SeqCst);
        let killer = self.running.lock().unwrap().clone();
        if let Some(killer) = killer {
            killer.kill_tree();
        }
    }

    pub(super) fn is_cancelled(&self, request: u64) -> bool {
        request <= self.cancelled_upto.load(Ordering::SeqCst)
    }

    /// Spawn `cmd` in its own process group / Job Object and record its
    /// [`TreeKiller`](crate::platform::process::TreeKiller), or refuse with
    /// `None` when `request` was cancelled. Checked and recorded under the lock,
    /// so a cancel either sees the killer or the spawn sees the cancel. A runner
    /// that is "Text file busy" for a moment, just installed or a test's freshly
    /// written stand-in, is waited out rather than reported as a failed run. The
    /// lock is let go between tries, so a cancel from the UI never waits on
    /// that, and the next try sees it.
    pub(super) fn spawn(&self, request: u64, cmd: &mut Command) -> Option<std::io::Result<Child>> {
        crate::review_ops::retry_while_busy(
            || {
                let mut running = self.running.lock().unwrap();
                if self.is_cancelled(request) {
                    return None;
                }
                match crate::platform::process::ManagedChild::spawn_in_group(cmd) {
                    Ok(managed) => {
                        let (child, killer) = managed.split_killer();
                        *running = Some(killer);
                        Some(Ok(child))
                    }
                    Err(e) => Some(Err(e)),
                }
            },
            |r| matches!(r, Some(Err(e)) if crate::review_ops::text_file_busy(e)),
        )
    }

    pub(super) fn finished(&self) {
        *self.running.lock().unwrap() = None;
    }
}
