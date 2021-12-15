use log::error;
use nix::unistd::getpid;
use std::{
    os::unix::prelude::ExitStatusExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, RwLock, RwLockReadGuard},
};
use thiserror::Error;
use tokio::{
    fs::File,
    io::{AsyncWriteExt, BufReader},
    process::Command,
    sync::{watch, Notify},
};
use uuid::Uuid;

use crate::{
    cgroup::PROC_FILE,
    runner::{self, job_status::Outcome, JobRequest},
    stack_string, Empty,
};
use nix::libc::{setgid, setuid};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    STString(#[from] stack_string::Error),
    #[error(transparent)]
    IOError(#[from] std::io::Error),
}

#[derive(Debug)]
pub enum JobStatus {
    Running,
    Exit(i32),
    Signal(i32),
}

impl<'a> From<RwLockReadGuard<'a, JobStatus>> for runner::JobStatus {
    fn from(value: RwLockReadGuard<'a, JobStatus>) -> Self {
        match *value {
            JobStatus::Running => runner::JobStatus { outcome: None },
            JobStatus::Exit(code) => runner::JobStatus {
                outcome: Some(Outcome::ExitCode(code)),
            },
            JobStatus::Signal(signal) => runner::JobStatus {
                outcome: Some(Outcome::Signal(signal)),
            },
        }
    }
}

impl Default for JobStatus {
    fn default() -> Self {
        JobStatus::Running
    }
}

#[derive(Debug)]
pub struct Job<S> {
    id: Uuid,
    cancel: Arc<Notify>,
    status: Arc<RwLock<JobStatus>>,
    state: S,
}

#[derive(Debug)]
pub struct Started {
    cgroup_dir: PathBuf,
    job_dir: PathBuf,
    completion: watch::Receiver<bool>,
}

pub struct Initialized;

impl Default for Job<Empty> {
    fn default() -> Self {
        let id = Uuid::new_v4();
        let cancel = Arc::new(Notify::new());
        let status = Arc::new(RwLock::new(JobStatus::default()));
        Self {
            id,
            cancel,
            status,
            state: Empty,
        }
    }
}
impl Job<Started> {
    pub fn job_dir(&self) -> &Path {
        &self.state.job_dir
    }

    pub fn cgroup_dir(&self) -> &Path {
        &self.state.cgroup_dir
    }

    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.state.completion.clone()
    }

    pub fn cancel(&self) {
        self.cancel.notify_one()
    }

    pub fn status(&self) -> runner::JobStatus {
        if !self.is_complete() {
            runner::JobStatus { outcome: None }
        } else {
            match self.status.read() {
                Ok(status) => status.into(),
                Err(err) => {
                    error!("Failed to read job status: {}", err);
                    runner::JobStatus { outcome: None }
                }
            }
        }
    }

    pub fn is_complete(&self) -> bool {
        match self.status.try_read() {
            Ok(status) => !matches!(*status, JobStatus::Running),
            // Either lock is poisoned (writer would not be able to continue)
            // Or WouldBlock (writer updates the status to complete and would not continue)
            Err(_) => true,
        }
    }
}

impl<T> Job<T> {
    pub fn id(&self) -> &Uuid {
        &self.id
    }
}

impl Job<Empty> {
    pub fn add_command(self, job_request: &JobRequest) -> Job<(Command, Empty, Empty, Empty)> {
        let Self {
            id, cancel, status, ..
        } = self;

        let mut handle = Command::new(&job_request.executable);
        handle
            .args(&job_request.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        Job {
            id,
            cancel,
            status,
            state: (handle, Empty, Empty, Empty),
        }
    }
}

impl<P, O, C> Job<(Command, P, O, C)> {
    pub fn set_job_dir(self, job_dir: PathBuf) -> Job<(Command, PathBuf, O, C)> {
        let Self {
            id,
            cancel,
            status,
            state,
            ..
        }: Job<(Command, P, O, C)> = self;
        let (mut cmd, _, ownership, cgroup) = state;
        cmd.current_dir(&job_dir);
        Job {
            id,
            cancel,
            status,
            state: (cmd, job_dir, ownership, cgroup),
        }
    }
}

impl<P> Job<(Command, P, Empty, Empty)> {
    pub fn add_to_cgroup(
        self,
        cgroup_path: PathBuf,
    ) -> Result<Job<(Command, P, Empty, PathBuf)>, Error> {
        let Self {
            id,
            cancel,
            status,
            state,
            ..
        } = self;
        let (mut cmd, job_dir, _, _) = state;
        let cgroup_procs: stack_string::String<256> = cgroup_path
            .join(PROC_FILE)
            // BASE_PATH is &str,
            // client is &str,
            // job_id is &str,
            // all &str's are valid utf8
            .to_str()
            .expect("Panic on valid utf8")
            .try_into()?;

        // Safety: all calls inside closure are async-signal-safe
        unsafe {
            cmd.pre_exec(move || {
                let pid: stack_string::String<11> = getpid().as_raw().into();
                std::fs::write(cgroup_procs, pid)?;
                Ok(())
            });
        }

        Ok(Job {
            id,
            cancel,
            status,
            state: (cmd, job_dir, Empty, cgroup_path),
        })
    }
}

impl Job<(Command, PathBuf, Initialized, PathBuf)> {
    pub fn spawn(self) -> Result<Job<Started>, Error> {
        let Self {
            id,
            cancel,
            status,
            state,
            ..
        } = self;
        let (mut cmd, job_dir, _, cgroup_dir, ..) = state;
        let mut child = cmd.spawn()?;

        // Unwrap: Command is instantiated by Job and is a private field
        // therefore out/error must be present
        let out = child.stdout.take().expect("Child stdout is missing");
        let err = child.stderr.take().expect("Child stderr is missing");

        let mut out = BufReader::new(out);
        let mut err = BufReader::new(err);

        let cancel_clone = cancel.clone();
        let status_clone = status.clone();

        let outfile = job_dir.join("out");
        let errfile = job_dir.join("err");

        let (tx, rx) = watch::channel(false);

        tokio::spawn(async move {
            let mut wout = File::create(outfile).await?;
            let mut werr = File::create(errfile).await?;

            loop {
                tokio::select! {
                    outcome = tokio::io::copy_buf(&mut out, &mut wout) => {
                        if let Err(err) = outcome {
                            error!("Error copying from child stdout to file: {}", err);
                        }
                    }
                    outcome = tokio::io::copy_buf(&mut err, &mut werr) => {
                        if let Err(err) = outcome {
                            error!("Error copying from child stderr to file: {}", err);
                        }
                    }
                    _ = cancel_clone.notified() => {
                        wout.flush().await?;
                        werr.flush().await?;
                        child.kill().await?;
                        // Can error only if RwLock is poisoned
                        // which cannot happen b/c it is the fist and only
                        // time this lock is used for writing;
                        let mut status = status_clone.write().unwrap();
                        *status = JobStatus::Signal(9);
                        break;
                    }
                    outcome = child.wait() => {
                        wout.flush().await?;
                        werr.flush().await?;
                        match outcome {
                            Ok(outcome) => {
                                if let Some(code) = outcome.code() {
                                    let mut status = status_clone.write().unwrap();
                                    *status = JobStatus::Exit(code);
                                } else {
                                    let signal = outcome.signal()
                                        .expect("Neither exit code nor signal");
                                    let mut status = status_clone.write().unwrap();
                                    *status = JobStatus::Signal(signal);
                                }
                            },

                            Err(outcome) => {
                                let exit_code = outcome.raw_os_error().unwrap_or_else(|| {
                                    error!("No exit code on child process failure: {}", outcome);
                                    -1
                                });

                                let mut status = status_clone.write().unwrap();
                                *status = JobStatus::Exit(exit_code);
                            },
                        }

                        break;
                    }
                }
            }

            if let Err(err) = tx.send(true) {
                error!(
                    "Failed to notify about job completion: {}, for job({})",
                    err, id
                );
            }

            Ok::<(), std::io::Error>(())
        });

        Ok(Job {
            id,
            cancel,
            status,
            state: Started {
                cgroup_dir,
                job_dir,
                completion: rx,
            },
        })
    }
}

impl<P> Job<(Command, P, Empty, PathBuf)> {
    pub fn set_ownership(self, uid: u32, gid: u32) -> Job<(Command, P, Initialized, PathBuf)> {
        let Self {
            id,
            cancel,
            status,
            state,
            ..
        } = self;
        let (mut cmd, job_dir, _, cgroup) = state;

        // Safety: all calls are async-signal-safe;
        unsafe {
            cmd.pre_exec(move || {
                match setgid(gid) {
                    0 => (),
                    x => return Err(std::io::Error::from_raw_os_error(x)),
                }

                match setuid(uid) {
                    0 => (),
                    x => return Err(std::io::Error::from_raw_os_error(x)),
                }

                Ok(())
            });
        }

        Job {
            id,
            cancel,
            status,
            state: (cmd, job_dir, Initialized, cgroup),
        }
    }
}
