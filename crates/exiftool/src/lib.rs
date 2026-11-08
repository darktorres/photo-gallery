//! ExifTool, kept running (`-stay_open`), for the writes that need its tags: a rating (`XMP:Rating`), an EXIF
//! orientation, and the metadata copied from a file to its copy. The program is the caller's (a path, or a name on
//! PATH); the process ends with this driver, and its console window is hidden (pg-platform).
//!
//! The file names go to ExifTool as their own bytes. Each argument is one line of its argument file, which carries
//! every name the probe tried (a leading space, a `#`, a raw non-UTF-8 byte, a lone surrogate on Windows), except one
//! with a line break or ending in whitespace: ExifTool reads such a line wrongly (see `needs_argv`), so a command with
//! such a name runs ExifTool once, with the names as its process arguments (exact on Unix; Windows can't hold such a
//! name).

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
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
    /// The program, for the commands that take the argument route.
    program: OsString,
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
        match ExifTool::pipes(&mut child) {
            Ok((stdin, stdout, stderr)) => {
                Ok(ExifTool { program: program.to_owned(), child, stdin, stdout, stderr, next: 1, broken: false })
            }
            // Started, but not usable: the process doesn't outlive the failure (a reader thread already started
            // ends at its pipe's end).
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(e)
            }
        }
    }

    /// A started ExifTool's stdin, and its stdout and stderr each read by a thread of its own (`pump`).
    fn pipes(child: &mut Child) -> Result<(ChildStdin, Receiver<String>, Receiver<String>)> {
        let stdin = child.stdin.take().context("ExifTool started without its stdin pipe")?;
        let stdout = pump(child.stdout.take().context("ExifTool started without its stdout pipe")?)?;
        let stderr = pump(child.stderr.take().context("ExifTool started without its stderr pipe")?)?;
        Ok((stdin, stdout, stderr))
    }

    /// Run one command on the argument file; returns (stdout, stderr). A failure (a pipe broken, or no answer in time)
    /// marks the process broken and kills it: a process that failed mid-command would answer the next command out of
    /// step.
    fn run(&mut self, args: &[&[u8]]) -> Result<(String, String)> {
        let result = self.exchange(args);
        if result.is_err() {
            self.broken = true;
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        result
    }

    fn exchange(&mut self, args: &[&[u8]]) -> Result<(String, String)> {
        let n = self.next;
        self.next += 1;
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        // File names as their own bytes (UTF-8 charset for them), one argument per line.
        let mut input = b"-charset\nfilename=utf8\n".to_vec();
        for a in args {
            input.extend_from_slice(a);
            input.push(b'\n');
        }
        input.extend_from_slice(format!("-echo4\n{{done{n}}}\n-execute{n}\n").as_bytes());
        self.stdin.write_all(&input)?;
        self.stdin.flush()?;
        let stdout = read_until(&self.stdout, &format!("{{ready{n}}}"), deadline)?;
        let stderr = read_until(&self.stderr, &format!("{{done{n}}}"), deadline)?;
        Ok((stdout, stderr))
    }

    /// One command: on the argument file, or as one process when an argument needs the argument route (`needs_argv`).
    fn exec(&mut self, args: &[&OsStr]) -> Result<(String, String)> {
        if needs_argv(args) {
            return oneshot(&self.program, args);
        }
        let bytes: Vec<&[u8]> = args.iter().map(|a| a.as_encoded_bytes()).collect();
        self.run(&bytes)
    }

    /// Write `XMP:Rating` into a copy of `path` at `out`. When ExifTool won't update the file as it is (damaged
    /// metadata), the metadata is rebuilt and the write retried once, as ImageRater did.
    pub fn write_rating(&mut self, path: &Path, out: &Path, rating: u8) -> Result<()> {
        let tag = format!("-XMP:Rating={rating}");
        let (stdout, stderr) = self.exec(&[OsStr::new(&tag), OsStr::new("-o"), out.as_os_str(), path.as_os_str()])?;
        if created(&stdout) {
            return Ok(());
        }
        let _ = std::fs::remove_file(out);
        let (stdout2, stderr2) = self.exec(&[
            OsStr::new("-all="),
            OsStr::new("-tagsfromfile"),
            OsStr::new("@"),
            OsStr::new("-all:all"),
            OsStr::new("-unsafe"),
            OsStr::new("-icc_profile"),
            OsStr::new(&tag),
            OsStr::new("-o"),
            out.as_os_str(),
            path.as_os_str(),
        ])?;
        if created(&stdout2) {
            return Ok(());
        }
        bail!("ExifTool couldn't write the rating into {}: {} {}", path.display(), stderr.trim(), stderr2.trim())
    }

    /// Copy every tag `from` holds that `to` can carry (EXIF, XMP, IPTC, ICC profile) into `to`, in place.
    pub fn copy_tags(&mut self, from: &Path, to: &Path) -> Result<()> {
        let (stdout, stderr) = self.exec(&[
            OsStr::new("-tagsfromfile"),
            from.as_os_str(),
            OsStr::new("-all:all"),
            OsStr::new("-unsafe"),
            OsStr::new("-icc_profile"),
            OsStr::new("-overwrite_original"),
            to.as_os_str(),
        ])?;
        if stdout.contains("1 image files updated") || stdout.contains("1 image files unchanged") {
            return Ok(());
        }
        bail!("ExifTool couldn't copy the metadata of {}: {}", from.display(), stderr.trim())
    }

    /// Write an EXIF orientation into a copy of `path` at `out` (adding EXIF if the file has none).
    pub fn write_orientation(&mut self, path: &Path, out: &Path, orientation: u8) -> Result<()> {
        let tag = format!("-EXIF:Orientation={orientation}");
        let (stdout, stderr) =
            self.exec(&[OsStr::new("-n"), OsStr::new(&tag), OsStr::new("-o"), out.as_os_str(), path.as_os_str()])?;
        if created(&stdout) {
            return Ok(());
        }
        bail!("ExifTool couldn't set the orientation of {}: {}", path.display(), stderr.trim())
    }
}

/// Whether an argument can't go through the argument file as it is. ExifTool reads a line break in an argument as the
/// end of the argument, and drops the whitespace at its end, so a name with either fails there ("File not found", or
/// "Can't create PNG files from scratch"). Probed on ExifTool 13.50 (WSL): a leading space or tab, and a CR, tab or
/// space inside a name, come through. Such an argument takes the argument route, which hands ExifTool the exact bytes on
/// Unix. A name ending in whitespace still can't be written by either route: ExifTool takes the type from the extension,
/// which then ends in whitespace (refused by the test of that name).
fn needs_argv(args: &[&OsStr]) -> bool {
    args.iter().any(|a| {
        let bytes = a.as_encoded_bytes();
        bytes.contains(&b'\n') || bytes.last().is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c))
    })
}

/// One ExifTool run with `args` as its process arguments (the names exact, on Unix). Its own process: the kept one is
/// not touched, so a failure here is the write's only.
fn oneshot(program: &OsStr, args: &[&OsStr]) -> Result<(String, String)> {
    let mut cmd = Command::new(program);
    cmd.args(["-charset", "filename=utf8"]).args(args).stdin(Stdio::null());
    pg_platform::process::hide_console_window(&mut cmd);
    let out = cmd.output().with_context(|| format!("starting {} (ExifTool)", program.to_string_lossy()))?;
    Ok((String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned()))
}

/// A pipe's lines, read by a thread of its own. ExifTool's stdout and stderr fill as it runs: read in turn, a full
/// stderr would stop it before the stdout marker that is read first. The thread ends at the pipe's end. A line is
/// read as bytes: one with no UTF-8 form (a name in ExifTool's message) is shown with U+FFFD, and the answer goes on.
/// A thread that can't be started is an error (`thread::spawn` would panic).
fn pump(pipe: impl Read + Send + 'static) -> Result<Receiver<String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("exiftool-pipe".into())
        .spawn(move || {
            let mut reader = BufReader::new(pipe);
            let mut line = Vec::new();
            loop {
                line.clear();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if tx.send(String::from_utf8_lossy(&line).into_owned()).is_err() {
                    break;
                }
            }
        })
        .context("starting a thread to read ExifTool's output")?;
    Ok(rx)
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

    /// The ExifTool at `program`, or None (the test is skipped) when it isn't there. `PG_REQUIRE_TOOLS` makes that a
    /// failure.
    fn start_or_skip(program: &OsStr) -> Option<ExifTool> {
        match ExifTool::start(program) {
            Ok(tool) => Some(tool),
            Err(_) => {
                assert!(
                    std::env::var_os("PG_REQUIRE_TOOLS").is_none(),
                    "ExifTool not found, and PG_REQUIRE_TOOLS is set"
                );
                eprintln!("ExifTool not found: skipped");
                None
            }
        }
    }

    /// Whether `bytes` hold `XMP:Rating` as 4 (ExifTool writes it as an attribute or an element, close after the name).
    fn rated_4(bytes: &[u8]) -> bool {
        bytes
            .windows(6)
            .enumerate()
            .any(|(i, w)| w == b"Rating" && bytes[i + 6..(i + 40).min(bytes.len())].contains(&b'4'))
    }

    /// The names the file tests write a file as: spaces, a leading space, a `#` or `-`, a quote, a `[CSTR]` look-alike
    /// and a lone surrogate (its bytes are raw bytes on Unix); and, on Unix only, a raw non-UTF-8 byte, a line break (the
    /// argument route), a carriage return inside a name, and a double quote, which Windows can't hold in a name. A name
    /// ending in whitespace is refused instead (`a_name_whose_extension_ends_in_whitespace_is_refused`).
    fn names() -> Vec<Vec<u8>> {
        let common = [
            b"plain.png".to_vec(),
            b"sp ace.png".to_vec(),
            b" lead.png".to_vec(),
            b"#hash.png".to_vec(),
            b"-dash.png".to_vec(),
            b"q'uote.png".to_vec(),
            b"[CSTR]x.png".to_vec(),
            b"w\xed\xa0\x80.png".to_vec(),
        ];
        #[cfg(unix)]
        let unix_only =
            vec![b"raw\xff.png".to_vec(), b"nl\nname.png".to_vec(), b"cr\rname.png".to_vec(), b"q\"uote.png".to_vec()];
        #[cfg(windows)]
        let unix_only: Vec<Vec<u8>> = Vec::new();
        common.into_iter().chain(unix_only).collect()
    }

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

    /// A line that isn't UTF-8 (ExifTool's message about a name with no UTF-8 form) doesn't end the answer: it is shown
    /// with U+FFFD, and the answer still ends at its marker.
    #[test]
    fn a_line_that_is_not_utf8_does_not_end_the_answer() {
        let lines = pump(std::io::Cursor::new(b"ab\xff\r\n{ready1}\n".to_vec())).unwrap();
        let answer = read_until(&lines, "{ready1}", Instant::now() + COMMAND_TIMEOUT).unwrap();
        assert_eq!(answer, "ab\u{FFFD}\n");
    }

    /// A name with a line break, or ending in whitespace, takes the argument route; the others go through the argument
    /// file as they are (the probe's cases, on either side of the rule).
    #[test]
    fn a_name_the_argument_file_cannot_carry_takes_the_argument_route() {
        for name in ["a\nb.jpg", "a.jpg\n", "a.jpg ", "a.jpg\t", "a.jpg\r", "a.jpg\x0b", "a.jpg\x0c"] {
            assert!(needs_argv(&[OsStr::new("-o"), OsStr::new(name)]), "{name:?} takes the argument route");
        }
        for name in ["a b.jpg", " lead.jpg", "a\rb.jpg", "a\tb.jpg", "#c'd\"e.jpg", "-dash.jpg", "[CSTR]x.jpg"] {
            assert!(!needs_argv(&[OsStr::new("-o"), OsStr::new(name)]), "{name:?} goes through the argument file");
        }
    }

    /// A process that died is replaced by the next command, instead of failing every later write. Needs
    /// ExifTool; skipped without it.
    #[test]
    fn a_dead_exiftool_is_replaced() {
        let program = OsString::from("exiftool");
        let Some(tool) = start_or_skip(&program) else { return };
        let mut slot = Some(tool);
        ExifTool::running(&mut slot, &program).unwrap();
        let tool = slot.as_mut().unwrap();
        tool.child.kill().unwrap();
        tool.child.wait().unwrap();
        assert!(tool.run(&[b"-ver".as_slice()]).is_err());
        assert!(slot.as_ref().unwrap().broken);

        ExifTool::running(&mut slot, &program).unwrap();
        let tool = slot.as_mut().unwrap();
        assert!(!tool.broken);
        assert!(tool.run(&[b"-ver".as_slice()]).is_ok());
    }

    /// A rating is written into a copy of a file of each name, the names' own bytes: the spaces, a leading `#` or `-`,
    /// a quote (Unix), a lone surrogate (Windows) or raw non-UTF-8 bytes (Unix), and a line break (Unix; the argument route).
    /// The original is left as it was. Needs ExifTool; skipped without it.
    #[test]
    fn a_rating_is_written_into_a_file_of_each_name() {
        let program = OsString::from("exiftool");
        let Some(mut tool) = start_or_skip(&program) else { return };
        let dir = pg_testkit::test_folder("scratch_exiftool", "names").unwrap();
        let source = pg_testkit::png_rgb(2, 2, &[0; 12]).unwrap();
        for (i, name) in names().iter().enumerate() {
            let path = dir.join(pg_platform::names::path_buf(name));
            std::fs::write(&path, &source).unwrap();
            let out = dir.join(format!("out-{i}.png"));
            tool.write_rating(&path, &out, 4).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
            let written = std::fs::read(&out).unwrap();
            assert!(rated_4(&written), "{}: the written copy has no rating 4", path.display());
            assert_eq!(std::fs::read(&path).unwrap(), source, "{}: the original changed", path.display());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A name whose last byte is whitespace or a CR can't be written by either route: the argument file trims it, and
    /// the argument route passes the exact bytes, but ExifTool then takes the type from the extension (`png `), which it
    /// doesn't write. The write fails with an error, no copy is left beside the original, and the original is unchanged.
    /// Unix only (Windows can't hold such a name). Needs ExifTool; skipped without it.
    #[cfg(unix)]
    #[test]
    fn a_name_whose_extension_ends_in_whitespace_is_refused_and_leaves_no_copy() {
        let program = OsString::from("exiftool");
        let Some(mut tool) = start_or_skip(&program) else { return };
        let dir = pg_testkit::test_folder("scratch_exiftool", "refused").unwrap();
        let source = pg_testkit::png_rgb(2, 2, &[0; 12]).unwrap();
        for name in [&b"trail.png "[..], &b"endcr.png\r"[..], &b"trailtab.png\t"[..]] {
            let path = dir.join(pg_platform::names::path_buf(name));
            std::fs::write(&path, &source).unwrap();
            let out = dir.join("refused-out.png");
            assert!(tool.write_rating(&path, &out, 4).is_err(), "{}: written", path.display());
            assert!(!out.exists(), "{}: a copy was left", path.display());
            assert_eq!(std::fs::read(&path).unwrap(), source, "{}: the original changed", path.display());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An orientation is written into a copy of a file of each name, and the tags of a rated file are copied into the
    /// file of each name in place, and out of it into a plain file: the names' own bytes, as the rating's test. The
    /// original of each copy is left as it was. Needs ExifTool; skipped without it.
    #[test]
    fn an_orientation_and_the_tags_are_written_for_a_file_of_each_name() {
        use pg_model::format::Format;
        use pg_model::orientation::Orientation;

        let program = OsString::from("exiftool");
        let Some(mut tool) = start_or_skip(&program) else { return };
        let dir = pg_testkit::test_folder("scratch_exiftool", "orientation-tags").unwrap();
        let source = pg_testkit::png_rgb(2, 2, &[0; 12]).unwrap();
        // A plain file with rating 4: the source of the tags.
        let input = dir.join("rated-in.png");
        std::fs::write(&input, &source).unwrap();
        let rated = dir.join("rated.png");
        tool.write_rating(&input, &rated, 4).unwrap();
        for (i, name) in names().iter().enumerate() {
            let path = dir.join(pg_platform::names::path_buf(name));
            std::fs::write(&path, &source).unwrap();
            // The orientation goes into a copy.
            let turned = dir.join(format!("turned-{i}.png"));
            tool.write_orientation(&path, &turned, 6).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
            let written = std::fs::read(&turned).unwrap();
            assert_eq!(
                pg_metadata::orientation(&written, Format::Png),
                Orientation::new(6).unwrap(),
                "{}: the written copy has no orientation 6",
                path.display()
            );
            assert_eq!(std::fs::read(&path).unwrap(), source, "{}: the original changed", path.display());
            // The rated file's tags go into the file, in place.
            tool.copy_tags(&rated, &path).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
            assert!(rated_4(&std::fs::read(&path).unwrap()), "{}: the copied tags have no rating 4", path.display());
            // And the file's tags go out of it, into a plain file.
            let plain = dir.join(format!("plain-{i}.png"));
            std::fs::write(&plain, &source).unwrap();
            tool.copy_tags(&path, &plain).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
            assert!(rated_4(&std::fs::read(&plain).unwrap()), "{}: nothing was copied out of it", path.display());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
