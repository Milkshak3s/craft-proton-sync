//! Runs Proton's Drive CLI: single commands, and a [`Session`] that keeps one CLI running in its
//! interactive mode for the many commands of a sync (a fresh CLI takes seconds to start).

use std::ffi::OsStr;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use craft_proton_sync_core::drive::{self, Kind, Node, PROMPT};
use craft_proton_sync_core::layout::REMOTE_ROOT;
use craft_proton_sync_core::plan::RemoteStamp;

use crate::lock;

#[derive(Clone, Debug)]
pub struct Cli {
    bin: PathBuf,
    /// The CLI's cache, data and log folder, inside the app's own.
    data: PathBuf,
}

impl Cli {
    pub fn new(bin: impl Into<PathBuf>, data: impl Into<PathBuf>) -> Self {
        Self {
            bin: bin.into(),
            data: data.into(),
        }
    }

    pub fn bin(&self) -> &Path {
        &self.bin
    }

    /// The CLI keeping its files in the app's folder, with no console window.
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.env("PROTON_DRIVE_CACHE_DIR", &self.data)
            .env("PROTON_DRIVE_LOG_LEVEL", "WARNING")
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd
    }

    pub fn output<I, S>(&self, args: I) -> Result<Output, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command()
            .args(args)
            .output()
            .map_err(|e| format!("couldn't run the Proton Drive CLI ({e})"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteError {
    /// Nobody is signed in, or the session ended.
    SignedOut,
    /// The CLI was stopped because the app is quitting.
    Stopped,
    NotFound,
    Failed(String),
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SignedOut => f.write_str("not signed in to Proton Drive"),
            Self::Stopped => f.write_str("stopped"),
            Self::NotFound => f.write_str("not found in Proton Drive"),
            Self::Failed(reason) => f.write_str(reason),
        }
    }
}

/// The synced folder in Proton Drive. Paths are relative to it, with forward slashes; the folder
/// itself is the empty path.
pub trait Remote {
    /// Makes sure the synced folder exists.
    fn ensure_root(&mut self) -> Result<(), RemoteError>;
    fn list(&mut self, dir: &str) -> Result<Vec<Node>, RemoteError>;
    fn create_folder(&mut self, parent: &str, name: &str) -> Result<(), RemoteError>;
    /// Uploads `local` into the folder `parent` as `name` (its own file name), over any file
    /// already there, and says what is there afterwards.
    fn upload(&mut self, local: &Path, parent: &str, name: &str) -> Result<RemoteStamp, RemoteError>;
    /// Downloads the file at `path` into the local folder `into`, under its own name.
    fn download(&mut self, path: &str, into: &Path) -> Result<(), RemoteError>;
    /// Moves the file at `path` to Proton Drive's trash. A file already gone is no error.
    fn trash(&mut self, path: &str) -> Result<(), RemoteError>;
}

/// How long to wait for the CLI's error message after it answered with nothing.
const ERROR_GRACE: Duration = Duration::from_millis(400);

/// One CLI in its interactive mode. The process sits in `slot` for as long as the session lives,
/// so that whoever quits the app can stop it.
pub struct Session<'a> {
    slot: &'a Mutex<Option<Child>>,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    /// What the CLI printed to standard error since the last command was sent.
    errors: Arc<Mutex<Vec<u8>>>,
}

impl<'a> Session<'a> {
    pub fn open(cli: &Cli, slot: &'a Mutex<Option<Child>>) -> Result<Self, RemoteError> {
        let failed = |what: &str| RemoteError::Failed(format!("couldn't run the Proton Drive CLI ({what})"));
        let mut child = cli
            .command()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| failed(&e.to_string()))?;
        let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        *lock(slot) = Some(child);
        let errors = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut stderr) = stderr {
            let errors = errors.clone();
            let reader = std::thread::Builder::new().name("drive-cli-errors".into()).spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n) = stderr.read(&mut buf) {
                    let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else {
                        break;
                    };
                    lock(&errors).extend_from_slice(chunk);
                }
            });
            if let Err(e) = reader {
                log::warn!("the Proton Drive CLI's errors won't be read ({e})");
            }
        }
        let Some(stdout) = stdout else { return Err(failed("no output")) };
        let mut session = Self {
            slot,
            stdin,
            stdout: BufReader::new(stdout),
            errors,
        };
        session.read_to_prompt()?;
        Ok(session)
    }

    /// What the CLI prints up to its next prompt.
    fn read_to_prompt(&mut self) -> Result<String, RemoteError> {
        let prompt = PROMPT.as_bytes();
        let mut out: Vec<u8> = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match self.stdout.read(&mut byte) {
                Ok(1) => out.extend_from_slice(&byte),
                _ => {
                    // The CLI is gone: stopped from outside, or it crashed.
                    return Err(if lock(self.slot).is_none() {
                        RemoteError::Stopped
                    } else {
                        self.error("the Proton Drive CLI stopped unexpectedly")
                    });
                }
            }
            // The prompt starts a line; inside an answer the same text could only be within a
            // JSON string, which can't start a line.
            if out.ends_with(prompt) {
                let before = out.len().saturating_sub(prompt.len());
                if before == 0 || out.get(before.wrapping_sub(1)) == Some(&b'\n') {
                    out.truncate(before);
                    return Ok(String::from_utf8_lossy(&out).into_owned());
                }
            }
        }
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&lock(&self.errors)).trim().to_string()
    }

    /// The error for a command that failed, from what the CLI said about it.
    fn error(&self, fallback: &str) -> RemoteError {
        let said = self.stderr();
        if drive::signed_out(&said) {
            RemoteError::SignedOut
        } else if drive::not_found(&said) {
            RemoteError::NotFound
        } else if said.is_empty() {
            RemoteError::Failed(fallback.to_string())
        } else {
            RemoteError::Failed(drive::friendly(&said))
        }
    }

    /// Sends one command and returns its JSON answer. `line` is `None` when a name in it can't be
    /// passed to the CLI.
    pub fn run(&mut self, line: Option<String>) -> Result<String, RemoteError> {
        let Some(line) = line else {
            return Err(RemoteError::Failed("the name has characters that can't be synced".into()));
        };
        lock(&self.errors).clear();
        let sent = self
            .stdin
            .as_mut()
            .map(|stdin| stdin.write_all(format!("{line}\n").as_bytes()).and_then(|()| stdin.flush()));
        if !matches!(sent, Some(Ok(()))) {
            return Err(if lock(self.slot).is_none() {
                RemoteError::Stopped
            } else {
                self.error("the Proton Drive CLI stopped unexpectedly")
            });
        }
        let answer = self.read_to_prompt()?;
        if !answer.trim().is_empty() {
            return Ok(answer);
        }
        // A failed command answers nothing and explains itself on standard error, which is read
        // on another thread and may land a moment later.
        let asked = Instant::now();
        while self.stderr().is_empty() && asked.elapsed() < ERROR_GRACE {
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(self.error("Proton Drive didn't answer"))
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        // Closing its input ends the CLI; no command is running at this point.
        self.stdin = None;
        if let Some(mut child) = lock(self.slot).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn failed(reason: String) -> RemoteError {
    RemoteError::Failed(reason)
}

impl Remote for Session<'_> {
    fn ensure_root(&mut self) -> Result<(), RemoteError> {
        let listed = self.run(drive::list_my_files())?;
        let exists = drive::parse_list(&listed)
            .map_err(failed)?
            .iter()
            .any(|n| n.name == REMOTE_ROOT && n.kind == Kind::Folder);
        if exists {
            return Ok(());
        }
        self.run(drive::create_root()).map(|_| ())
    }

    fn list(&mut self, dir: &str) -> Result<Vec<Node>, RemoteError> {
        let listed = self.run(drive::list(dir))?;
        drive::parse_list(&listed).map_err(failed)
    }

    fn create_folder(&mut self, parent: &str, name: &str) -> Result<(), RemoteError> {
        match self.run(drive::create_folder(parent, name)) {
            Err(RemoteError::Failed(e)) if drive::already_exists(&e) => Ok(()),
            other => other.map(|_| ()),
        }
    }

    fn upload(&mut self, local: &Path, parent: &str, name: &str) -> Result<RemoteStamp, RemoteError> {
        let answer = self.run(local.to_str().and_then(|local| drive::upload(local, parent)))?;
        drive::parse_transfer(&answer).map_err(failed)?;
        let path = if parent.is_empty() {
            name.to_string()
        } else {
            format!("{parent}/{name}")
        };
        let info = self.run(drive::info(&path))?;
        match drive::parse_info(&info).map_err(failed)?.kind {
            Kind::File { stamp, .. } => Ok(stamp),
            Kind::Folder => Err(failed("a folder with the file's name is in the way in Proton Drive".into())),
        }
    }

    fn download(&mut self, path: &str, into: &Path) -> Result<(), RemoteError> {
        let answer = self.run(into.to_str().and_then(|into| drive::download(path, into)))?;
        drive::parse_transfer(&answer).map_err(failed)
    }

    fn trash(&mut self, path: &str) -> Result<(), RemoteError> {
        match self.run(drive::trash(path)) {
            Err(RemoteError::NotFound) => Ok(()),
            Err(e) => Err(e),
            Ok(answer) => drive::parse_trash(&answer).map_err(failed),
        }
    }
}
