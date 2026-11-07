//! ExifTool, kept running (`-stay_open`), for the writes that need its tags: a rating (`XMP:Rating`), an EXIF
//! orientation, and the metadata copied from a file to its copy. The program is the caller's (a path, or a name on
//! PATH); the process ends with this driver, and its console window is hidden (pg-platform).

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// How long one command may take, from its first byte to its last marker. A command past it fails, and its process is
/// killed: the next command starts a new one.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a closing ExifTool gets to exit after `-stay_open False` before it is killed.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// One ExifTool kept running (`-stay_open`), as starting it costs ~0.3 s per call.
pub struct ExifTool {
    child: Child,
    stdin: ChildStdin,
    /// Each pipe's lines, read by a thread of its own (`pump`).
    stdout: Receiver<String>,
    stderr: Receiver<String>,
    next: u64,
    /// A command's pipe failed (the process exited or its pipes broke): this one can't be used again.
    broken: bool,
}

impl ExifTool {
    /// The running ExifTool in `slot`, starting one if the slot is empty or its process has failed. A failed
    /// command is an error for that write only: the next one gets a new process.
    pub fn running<'a>(slot: &'a mut Option<ExifTool>, program: &OsStr) -> Result<&'a mut ExifTool> {
        let tool = match slot.take() {
            Some(t) if !t.broken => t,
            _ => ExifTool::start(program)?,
        };
        Ok(slot.insert(tool))
    }

    /// The ExifTool at `program` (a path, or a name on PATH), started for the commands to come.
    pub fn start(program: &OsStr) -> Result<ExifTool> {
        let mut cmd = Command::new(program);
        cmd.args(["-stay_open", "True", "-@", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        pg_platform::process::hide_console_window(&mut cmd);
        let mut child = cmd.spawn().with_context(|| {
            format!("starting {} (ExifTool, needed to write ratings into files)", program.to_string_lossy())
        })?;
        pg_platform::process::die_with_parent(&child);
        let stdin = child.stdin.take().context("ExifTool started without its stdin pipe")?;
        let stdout = pump(child.stdout.take().context("ExifTool started without its stdout pipe")?);
        let stderr = pump(child.stderr.take().context("ExifTool started without its stderr pipe")?);
        Ok(ExifTool { child, stdin, stdout, stderr, next: 1, broken: false })
    }

    /// Run one command; returns (stdout, stderr). A failure (a pipe broken, or no answer in time) marks the process
    /// broken and kills it: a process that failed mid-command would answer the next command out of step.
    fn run(&mut self, args: &[&str]) -> Result<(String, String)> {
        let result = self.exchange(args);
        if result.is_err() {
            self.broken = true;
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        result
    }

    fn exchange(&mut self, args: &[&str]) -> Result<(String, String)> {
        let n = self.next;
        self.next += 1;
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        // UTF-8 file names (non-ASCII paths), one argument per line.
        let mut input = String::from("-charset\nfilename=utf8\n");
        for a in args {
            input.push_str(a);
            input.push('\n');
        }
        input.push_str(&format!("-echo4\n{{done{n}}}\n-execute{n}\n"));
        self.stdin.write_all(input.as_bytes())?;
        self.stdin.flush()?;
        let stdout = read_until(&self.stdout, &format!("{{ready{n}}}"), deadline)?;
        let stderr = read_until(&self.stderr, &format!("{{done{n}}}"), deadline)?;
        Ok((stdout, stderr))
    }

    /// Write `XMP:Rating` into a copy of `path` at `out`. When ExifTool won't update the file as it is (damaged
    /// metadata), the metadata is rebuilt and the write retried once, as ImageRater did.
    pub fn write_rating(&mut self, path: &str, out: &str, rating: u8) -> Result<()> {
        let tag = format!("-XMP:Rating={rating}");
        let (stdout, stderr) = self.run(&[&tag, "-o", out, path])?;
        if created(&stdout) {
            return Ok(());
        }
        let _ = std::fs::remove_file(out);
        let (stdout2, stderr2) =
            self.run(&["-all=", "-tagsfromfile", "@", "-all:all", "-unsafe", "-icc_profile", &tag, "-o", out, path])?;
        if created(&stdout2) {
            return Ok(());
        }
        bail!("ExifTool couldn't write the rating into {path}: {} {}", stderr.trim(), stderr2.trim())
    }

    /// Copy every tag `from` holds that `to` can carry (EXIF, XMP, IPTC, ICC profile) into `to`, in place.
    pub fn copy_tags(&mut self, from: &str, to: &str) -> Result<()> {
        let (stdout, stderr) =
            self.run(&["-tagsfromfile", from, "-all:all", "-unsafe", "-icc_profile", "-overwrite_original", to])?;
        if stdout.contains("1 image files updated") || stdout.contains("1 image files unchanged") {
            return Ok(());
        }
        bail!("ExifTool couldn't copy the metadata of {from}: {}", stderr.trim())
    }

    /// Write an EXIF orientation into a copy of `path` at `out` (adding EXIF if the file has none).
    pub fn write_orientation(&mut self, path: &str, out: &str, orientation: u8) -> Result<()> {
        let tag = format!("-EXIF:Orientation={orientation}");
        let (stdout, stderr) = self.run(&["-n", &tag, "-o", out, path])?;
        if created(&stdout) {
            return Ok(());
        }
        bail!("ExifTool couldn't set the orientation of {path}: {}", stderr.trim())
    }
}

/// A pipe's lines, read by a thread of its own. ExifTool's stdout and stderr fill as it runs: read in turn, a full
/// stderr would stop it before the stdout marker that is read first. The thread ends at the pipe's end.
fn pump(pipe: impl Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// The lines up to `marker` (not included). An ExifTool that exited ends the wait, and one that hasn't reached the
/// marker by `deadline` fails it.
fn read_until(lines: &Receiver<String>, marker: &str, deadline: Instant) -> Result<String> {
    let mut out = String::new();
    loop {
        let wait = deadline.checked_duration_since(Instant::now()).unwrap_or_default();
        let line = match lines.recv_timeout(wait) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => {
                bail!("ExifTool didn't answer within {}s", COMMAND_TIMEOUT.as_secs())
            }
            Err(RecvTimeoutError::Disconnected) => bail!("ExifTool exited"),
        };
        if line.trim_end() == marker {
            return Ok(out);
        }
        out.push_str(&line);
        out.push('\n');
    }
}

/// "1 image files created" (with `-o`).
fn created(stdout: &str) -> bool {
    stdout.lines().any(|l| l.trim().starts_with("1 image files created"))
}

impl Drop for ExifTool {
    /// Asks ExifTool to exit, and kills it when it hasn't within `CLOSE_TIMEOUT`: a process stuck on a command must
    /// not hold the app's shutdown.
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"-stay_open\nFalse\n");
        let _ = self.stdin.flush();
        let deadline = Instant::now() + CLOSE_TIMEOUT;
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command's lines up to its marker are its answer; a marker that doesn't come by the deadline fails it.
    #[test]
    fn a_command_is_read_up_to_its_marker_or_times_out() {
        let (tx, rx) = mpsc::channel::<String>();
        for line in ["1 image files created", "{ready1}"] {
            tx.send(line.to_owned()).unwrap();
        }
        let answer = read_until(&rx, "{ready1}", Instant::now() + COMMAND_TIMEOUT).unwrap();
        assert_eq!(answer, "1 image files created\n");

        let err = read_until(&rx, "{ready2}", Instant::now() + Duration::from_millis(50)).unwrap_err();
        assert!(err.to_string().contains("didn't answer"), "{err}");
        drop(tx);
        let err = read_until(&rx, "{ready2}", Instant::now() + COMMAND_TIMEOUT).unwrap_err();
        assert!(err.to_string().contains("exited"), "{err}");
    }

    /// A process that died is replaced by the next command, instead of failing every later write. Needs
    /// ExifTool; skipped without it.
    #[test]
    fn a_dead_exiftool_is_replaced() {
        let program = std::ffi::OsString::from("exiftool");
        let mut slot = match ExifTool::start(&program) {
            Ok(tool) => Some(tool),
            Err(_) => {
                assert!(
                    std::env::var_os("PG_REQUIRE_TOOLS").is_none(),
                    "ExifTool not found, and PG_REQUIRE_TOOLS is set"
                );
                eprintln!("ExifTool not found: skipped");
                return;
            }
        };
        ExifTool::running(&mut slot, &program).unwrap();
        let tool = slot.as_mut().unwrap();
        tool.child.kill().unwrap();
        tool.child.wait().unwrap();
        assert!(tool.run(&["-ver"]).is_err());
        assert!(slot.as_ref().unwrap().broken);

        ExifTool::running(&mut slot, &program).unwrap();
        let tool = slot.as_mut().unwrap();
        assert!(!tool.broken);
        assert!(tool.run(&["-ver"]).is_ok());
    }
}
