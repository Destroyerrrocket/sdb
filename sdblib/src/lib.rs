#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::complexity)]
#![warn(clippy::correctness)]
#![warn(clippy::nursery)]
#![warn(clippy::perf)]
#![warn(clippy::style)]
#![warn(clippy::suspicious)]

use std::os::unix::process::CommandExt;
use thiserror::Error;
use tracing::{Level, event, instrument};

#[derive(Error, Debug)]
pub enum DebuggerError {
    #[error("IO Error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Nix Error: {0}")]
    NixError(#[from] nix::errno::Errno),
    #[error("Error: {0}")]
    ErrorMessage(String),
    #[error("Unknown Error")]
    Unknown,
}

#[derive(Debug)]
pub struct Debugger {
    managed_processes: Vec<std::process::Child>,

    attached_processes: Vec<nix::unistd::Pid>,
}

impl Default for Debugger {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Debugger {
    fn drop(&mut self) {
        for child in &mut self.managed_processes {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Debugger {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            managed_processes: Vec::new(),
            attached_processes: Vec::new(),
        }
    }

    #[instrument]
    /// Attach a process into the debugger by its PID.
    /// # Errors
    ///
    /// Will return `Err` if `PID` does not exist or is invalid.
    pub fn add_proc(&mut self, pid: u64) -> Result<(), DebuggerError> {
        let pid = nix::unistd::Pid::from_raw(
            pid.try_into()
                .map_err(|e| DebuggerError::ErrorMessage(format!("PID conversion error: {e}")))?,
        );
        event!(Level::INFO, "Adding process with PID: {}", pid);
        nix::sys::ptrace::attach(pid)?;
        self.attached_processes.push(pid);
        Ok(())
    }

    #[instrument]
    /// Run a program under the debugger with given arguments.
    /// # Errors
    ///
    /// Will return `Err` if the program fails to start, or we fail to attach.
    pub fn add_program<I, S>(
        &mut self,
        program: &str,
        args: I,
    ) -> Result<std::process::ChildStdout, DebuggerError>
    where
        I: IntoIterator<Item = S> + std::fmt::Debug,
        S: AsRef<std::ffi::OsStr>,
    {
        event!(Level::INFO, "Adding program: {}", program);
        let mut child = unsafe {
            std::process::Command::new(program)
                .args(args)
                .pre_exec(|| -> std::io::Result<()> {
                    nix::sys::ptrace::traceme()?;
                    Ok(())
                })
                .stdout(std::process::Stdio::piped())
                .spawn()?
        };
        self.attached_processes
            .push(nix::unistd::Pid::from_raw(child.id().cast_signed()));
        let stdout = child.stdout.take().ok_or_else(|| {
            DebuggerError::ErrorMessage("Failed to take stdout of the child process".to_string())
        });
        self.managed_processes.push(child);
        stdout
    }

    #[instrument]
    /// Waits for all attached processes to change state.
    /// # Errors
    ///
    /// Will return `Err` if the program no longer exists.
    pub fn wait(&self) -> Result<(), DebuggerError> {
        for pid in &self.attached_processes {
            nix::sys::wait::waitpid(*pid, None)?;
        }
        Ok(())
    }

    #[instrument]
    /// Continues the execution of all attached processes.
    /// # Errors
    ///
    /// Will return `Err` if the program was already running or has exited.
    pub fn continue_execution(&self) -> Result<(), DebuggerError> {
        for pid in &self.attached_processes {
            nix::sys::ptrace::cont(*pid, None)?;
        }
        Ok(())
    }
}
