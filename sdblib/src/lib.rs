use std::os::unix::process::CommandExt;

use tracing::{Level, event, instrument};

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
    pub fn new() -> Self {
        Debugger {
            managed_processes: Vec::new(),
            attached_processes: Vec::new(),
        }
    }

    #[instrument]
    pub fn add_proc(&mut self, pid: u64) {
        event!(Level::INFO, "Adding process with PID: {}", pid);
        nix::sys::ptrace::attach(nix::unistd::Pid::from_raw(pid.try_into().unwrap())).unwrap();
        self.attached_processes
            .push(nix::unistd::Pid::from_raw(pid.try_into().unwrap()));
    }

    #[instrument]
    pub fn add_program<I, S>(&mut self, program: &str, args: I)
    where
        I: IntoIterator<Item = S> + std::fmt::Debug,
        S: AsRef<std::ffi::OsStr>,
    {
        event!(Level::INFO, "Adding program: {}", program);
        let child = unsafe {
            std::process::Command::new(program)
                .args(args)
                .pre_exec(|| -> std::io::Result<()> {
                    nix::sys::ptrace::traceme()?;
                    Ok(())
                })
                .spawn()
                .expect("Failed to start program")
        };
        self.attached_processes
            .push(nix::unistd::Pid::from_raw(child.id() as i32));
        self.managed_processes.push(child);
    }

    pub fn wait(&self) {
        for pid in &self.attached_processes {
            nix::sys::wait::waitpid(*pid, None).unwrap();
        }
    }

    pub fn continue_execution(&self) {
        for pid in &self.attached_processes {
            nix::sys::ptrace::cont(*pid, None).unwrap();
        }
        self.wait();
    }
}
