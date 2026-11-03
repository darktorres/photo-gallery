//! ExifTool, kept running (`-stay_open`), for the writes that need its tags: a rating (`XMP:Rating`), an EXIF
//! orientation, and the metadata copied from a file to its copy. The program is the caller's (a path, or a name on
//! PATH); the process ends with this driver, and its console window is hidden (pg-platform).

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};

use anyhow::{Context, Result, bail};

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
    /// The running ExifTool in `slot`, starting one if the slot is empty or its process has failed (R-38). A failed
    /// command is an error for that write only: the next one gets a new process.
    pub fn running<'a>(slot: &'a mut Option<ExifTool>, program: &OsStr) -> Result<&'a mut ExifTool> {
        if slot.as_ref().is_none_or(|t| t.broken) {
            *slot = Some(ExifTool::start(program)?);
        }
        Ok(slot.as_mut().unwrap())
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
        let stdin = child.stdin.take().unwrap();
        let stdout = pump(child.stdout.take().unwrap());
        let stderr = pump(child.stderr.take().unwrap());
        Ok(ExifTool { child, stdin, stdout, stderr, next: 1, broken: false })
    }

    /// Run one command; returns (stdout, stderr). A pipe failure marks the process broken.
    fn run(&mut self, args: &[&str]) -> Result<(String, String)> {
        let result = self.exchange(args);
        if result.is_err() {
            self.broken = true;
        }
        result
    }

    fn exchange(&mut self, args: &[&str]) -> Result<(String, String)> {
        let n = self.next;
        self.next += 1;
        // UTF-8 file names (non-ASCII paths), one argument per line.
        let mut input = String::from("-charset\nfilename=utf8\n");
        for a in args {
            input.push_str(a);
            input.push('\n');
        }
        input.push_str(&format!("-echo4\n{{done{n}}}\n-execute{n}\n"));
        self.stdin.write_all(input.as_bytes())?;
        self.stdin.flush()?;
        let stdout = read_until(&self.stdout, &format!("{{ready{n}}}"))?;
        let stderr = read_until(&self.stderr, &format!("{{done{n}}}"))?;
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
/// stderr would stop it before the stdout marker that is read first (R-39). The thread ends at the pipe's end.
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

/// The lines up to `marker` (not included). An ExifTool that exited ends the wait.
fn read_until(lines: &Receiver<String>, marker: &str) -> Result<String> {
    let mut out = String::new();
    loop {
        let line = lines.recv().map_err(|_| anyhow::anyhow!("ExifTool exited"))?;
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
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"-stay_open\nFalse\n");
        let _ = self.stdin.flush();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A process that died is replaced by the next command, instead of failing every later write (R-38). Needs
    /// ExifTool; skipped without it.
    #[test]
    fn a_dead_exiftool_is_replaced() {
        let program = std::ffi::OsString::from("exiftool");
        let mut slot = match ExifTool::start(&program) {
            Ok(tool) => Some(tool),
            Err(_) => {
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
