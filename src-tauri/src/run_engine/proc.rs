//! Process-group control: each run's process leads its own group, so one signal reaches the tools it
//! spawned.

use std::time::Duration;

use crate::ai::Harness;

/// How long a terminated group gets before it is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(5);

#[cfg(unix)]
pub fn process_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes for existence.
    unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

#[cfg(not(unix))]
pub fn process_alive(_pid: i32) -> bool {
    false
}

#[cfg(unix)]
pub fn signal_group(pgid: i32, sig: i32) {
    // SAFETY: plain syscall; a stale pgid at worst yields ESRCH.
    unsafe {
        libc::kill(-pgid, sig);
    }
}

#[cfg(not(unix))]
pub fn signal_group(_pgid: i32, _sig: i32) {}

#[cfg(unix)]
pub const SIGTERM: i32 = libc::SIGTERM;
#[cfg(unix)]
pub const SIGKILL: i32 = libc::SIGKILL;
#[cfg(not(unix))]
pub const SIGTERM: i32 = 15;
#[cfg(not(unix))]
pub const SIGKILL: i32 = 9;

/// SIGTERM the group (the agent ends its bash children and exits 143), SIGKILL it if it lingers.
pub fn terminate(pgid: i32) {
    signal_group(pgid, SIGTERM);
    std::thread::spawn(move || {
        std::thread::sleep(STOP_GRACE);
        signal_group(pgid, SIGKILL);
    });
}

/// SIGTERM a process group left by an earlier session, when it still looks like the harness' own.
pub fn kill_leftover(harness: &dyn Harness, pid: i32) {
    if harness.is_own_process(pid, None) {
        signal_group(pid, SIGTERM);
    }
}
