//! Child processes. ExifTool, which writes ratings and orientations, runs without a console window and ends with the process
//! that started it: `spawn_background` arranges that (a job object on Windows, a parent-death signal on Linux).

use std::io;
use std::process::{Child, Command};

/// Keep a child process from opening a console window (on Windows).
#[cfg(windows)]
pub fn hide_console_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// CREATE_NO_WINDOW.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// Elsewhere there is no console window to keep from opening.
#[cfg(not(windows))]
pub fn hide_console_window(_cmd: &mut Command) {}

/// Starts `cmd` as a child process that ends when this process does, however it ends: the app quitting runs no
/// destructors, so a stay-open ExifTool would otherwise outlive it.
#[cfg(windows)]
pub fn spawn_background(mut cmd: Command) -> io::Result<Child> {
    let child = cmd.spawn()?;
    die_with_parent(&child);
    Ok(child)
}

/// Starts `cmd` as a child that the kernel ends when this process does, however it ends. Linux's parent-death signal is
/// sent when the thread that started the child ends, so the starts are made on one thread that lives as long as the
/// process (`spawner`): a child started on a short-lived thread would be killed when that thread ended.
#[cfg(target_os = "linux")]
pub fn spawn_background(cmd: Command) -> io::Result<Child> {
    use std::sync::mpsc;
    let (reply, result) = mpsc::channel();
    let gone = || io::Error::other("the child spawner thread has ended");
    spawner()?.send((cmd, reply)).map_err(|_| gone())?;
    result.recv().map_err(|_| gone())?
}

/// Elsewhere nothing ends a child with this process. The app's own exit ends a stay-open ExifTool through its `Drop`;
/// after a crash, a stay-open ExifTool keeps running, since it does not end when its pipes close.
#[cfg(all(not(windows), not(target_os = "linux")))]
pub fn spawn_background(mut cmd: Command) -> io::Result<Child> {
    cmd.spawn()
}

/// A start for the spawner thread: the command, and the channel its child (or its error) goes back on.
#[cfg(target_os = "linux")]
type Start = (Command, std::sync::mpsc::Sender<io::Result<Child>>);

/// The queue to the spawner thread, which starts on first use. If the thread can't start, each start is an error.
#[cfg(target_os = "linux")]
fn spawner() -> io::Result<&'static std::sync::mpsc::Sender<Start>> {
    use std::sync::{OnceLock, mpsc};
    static SPAWNER: OnceLock<Result<mpsc::Sender<Start>, String>> = OnceLock::new();
    SPAWNER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Start>();
            std::thread::Builder::new()
                .name("child spawner".to_string())
                .spawn(move || {
                    for (mut cmd, reply) in rx {
                        // The caller has gone only if its reply can't be sent, and then nothing is left to tell it.
                        let _ = reply.send(spawn_with_parent_death(&mut cmd));
                    }
                })
                .map(|_| tx)
                .map_err(|e| format!("starting the child spawner thread: {e}"))
        })
        .as_ref()
        .map_err(|e| io::Error::other(e.clone()))
}

/// Starts `cmd` with the parent-death signal set to SIGKILL, on the calling thread.
#[cfg(target_os = "linux")]
fn spawn_with_parent_death(cmd: &mut Command) -> io::Result<Child> {
    use std::os::unix::process::CommandExt;
    // SAFETY: getpid cannot fail.
    let parent = unsafe { libc::getpid() };
    // SAFETY: the closure runs in the forked child before exec, where only async-signal-safe calls are allowed. prctl and
    // getppid are, and the closure does not allocate: its error is a raw OS error.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) != 0 {
                return Err(io::Error::last_os_error());
            }
            // The parent may have ended between the fork and the signal being set, when the signal could not fire. The
            // child is then already an orphan, so it fails to start rather than run on.
            if libc::getppid() != parent {
                return Err(io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
    cmd.spawn()
}

/// Put a child process in a job that kills it when this process ends, however it ends.
#[cfg(windows)]
fn die_with_parent(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    // The job handle is never closed: the system closes it when the process exits.
    static JOB: OnceLock<usize> = OnceLock::new();
    let job = *JOB.get_or_init(|| {
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // A size the call can't take (it can't happen) is a limit not set, as when the call fails.
        let size = u32::try_from(std::mem::size_of_val(&info));
        // SAFETY: null security attributes and name are allowed; `info` is the struct this information class
        // takes, with its size.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if let Ok(size) = size {
                SetInformationJobObject(job, JobObjectExtendedLimitInformation, (&raw const info).cast(), size);
            }
            job.addr()
        }
    });
    // The stored address becomes a handle again. A handle is a number the kernel gives, not memory: no provenance.
    let job = std::ptr::without_provenance_mut(job);
    // SAFETY: the job handle is never closed, and the child's handle is open while `child` lives.
    unsafe { AssignProcessToJobObject(job, child.as_raw_handle()) };
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    /// `sleep` for `secs` seconds, with no input or output of its own.
    fn sleeper(secs: u32) -> Command {
        let mut cmd = Command::new("sleep");
        cmd.arg(secs.to_string()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        cmd
    }

    /// Whether `pid` is still a process that runs or waits. A zombie (ended, not yet reaped) is not.
    fn alive(pid: u32) -> bool {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // The state is the first field after the command name, which is in parentheses and may hold spaces.
            Ok(stat) => !stat.rsplit(')').next().unwrap_or_default().trim_start().starts_with('Z'),
            Err(_) => false,
        }
    }

    #[test]
    fn a_child_outlives_the_thread_that_started_it() {
        let mut child = std::thread::spawn(|| spawn_background(sleeper(30))).join().unwrap().unwrap();
        // The thread has ended; the child has not.
        std::thread::sleep(Duration::from_millis(200));
        assert!(alive(child.id()), "the child ended with the thread that started it");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    /// The helper for the next test: starts a child and exits at once, without running destructors. It does nothing
    /// unless the test below runs it in a process of its own.
    #[test]
    fn spawn_helper() {
        if std::env::var_os("PG_PLATFORM_SPAWN_HELPER").is_none() {
            return;
        }
        let child = spawn_background(sleeper(60)).unwrap();
        println!("pid={}", child.id());
        std::process::exit(0);
    }

    #[test]
    fn a_child_ends_with_the_process_that_started_it() {
        let exe = std::env::current_exe().unwrap();
        let out = std::process::Command::new(exe)
            .args(["--exact", "process::tests::spawn_helper", "--nocapture"])
            .env("PG_PLATFORM_SPAWN_HELPER", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let pid: u32 = stdout
            .lines()
            .find_map(|line| line.strip_prefix("pid="))
            .and_then(|pid| pid.trim().parse().ok())
            .expect("the helper prints its child's pid");
        // The helper has exited; its child must end with it, within a few seconds.
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let outlived = alive(pid);
        if outlived {
            let _ = std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).status();
        }
        assert!(!outlived, "the child {pid} outlived the process that started it");
    }
}
