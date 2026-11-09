//! An opt-in time limit for the `git` / `gh` children of the current thread. The app never sets
//! one (its calls behave as before); `cr worker` wraps workspace resolution in [`with_timeout`] so a
//! hung `gh` or `git` cannot hold a daemon lock forever.

use std::cell::Cell;
use std::io::{self, Read};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

thread_local! {
    static LIMIT: Cell<Option<Duration>> = const { Cell::new(None) };
}

/// Run `f` with every [`output`] call on this thread limited to `limit`.
pub fn with_timeout<T>(limit: Duration, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Duration>);
    impl Drop for Restore {
        fn drop(&mut self) {
            LIMIT.with(|l| l.set(self.0));
        }
    }
    let _restore = Restore(LIMIT.with(|l| l.replace(Some(limit))));
    f()
}

pub fn is_timeout(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::TimedOut
}

fn pipe<R: Read + Send + 'static>(r: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut r) = r {
            let _ = r.read_to_end(&mut buf);
        }
        buf
    })
}

/// `cmd.output()`, killed (with its process group) once the thread's limit passes.
pub fn output(cmd: &mut Command) -> io::Result<Output> {
    let Some(limit) = LIMIT.with(|l| l.get()) else { return cmd.output() };
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(cmd, 0);
    let mut child = cmd.spawn()?;
    let out = pipe(child.stdout.take());
    let err = pipe(child.stderr.take());
    let end = Instant::now() + limit;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break Some(s);
        }
        if Instant::now() >= end {
            #[cfg(unix)]
            // SAFETY: plain syscall on the group this call created; a stale pgid yields ESRCH.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    match status {
        Some(status) => Ok(Output { status, stdout, stderr }),
        None => Err(io::Error::new(io::ErrorKind::TimedOut, format!("timed out after {}s", limit.as_secs().max(1)))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)] // a killed shell does not take its grandchild (and the pipe it holds) with it on Windows
    #[test]
    fn a_hung_child_is_killed_at_the_limit() {
        let started = Instant::now();
        let r = with_timeout(Duration::from_millis(200), || output(Command::new("sh").args(["-c", "sleep 30 & sleep 30"])));
        assert!(is_timeout(&r.unwrap_err()));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn without_a_limit_or_within_it_the_output_is_returned() {
        let r = output(Command::new("sh").args(["-c", "echo hi"])).unwrap();
        assert_eq!(r.stdout, b"hi\n");
        let r = with_timeout(Duration::from_secs(10), || output(Command::new("sh").args(["-c", "echo hi"]))).unwrap();
        assert_eq!(r.stdout, b"hi\n");
        assert!(LIMIT.with(|l| l.get()).is_none(), "the limit is restored");
    }
}
