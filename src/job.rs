use std::{sync::{Arc, RwLock}, process::Stdio, path::Path, os::unix::prelude::ExitStatusExt};
use log::error;
use nix::unistd::getpid;
use tokio::{sync::Notify, process::Command, io::{BufReader, BufWriter, AsyncWriteExt}, fs::File};
use uuid::Uuid;
use thiserror::Error;

use crate::{Empty, runner::JobRequest, stack_string, cgroup::PROC_FILE};

#[cfg(target_os = "linux")]
use nix::libc::{setgid, setuid};

#[cfg(not(target_os = "linux"))]
use crate::stub::{setuid, setgid};

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

impl Default for JobStatus {
    fn default() -> Self {
        JobStatus::Running
    }
}

#[derive(Debug)]
pub struct Job<H> {
    id: Uuid,
    cancel: Arc<Notify>,
    status: Arc<RwLock<JobStatus>>,
    handle: H,
}

#[derive(Debug)]
pub struct Started;
pub struct Initialized;

impl Default for Job<Empty> {
    fn default() -> Self {
        let id = Uuid::new_v4();
        let cancel = Arc::new(Notify::new());
        let status = Arc::new(RwLock::new(JobStatus::default()));
        Self { id, cancel, status, handle: Empty }
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
            id,
            cancel,
            status,
            ..
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
            handle: (handle, Empty, Empty, Empty),
        }
    }
}

impl<P, O, C> Job<(Command, P, O, C)> {
    pub fn set_job_dir(self, job_dir: &Path) -> Job<(Command, &Path, O, C)> {
        let Self { id, cancel, status, handle, ..}: Job<(Command, P, O, C)> = self;
        let (mut cmd, _, ownership, cgroup) = handle;
        cmd.current_dir(&job_dir);
        Job {
            id,
            cancel,
            status,
            handle: (cmd, job_dir, ownership, cgroup),
        }
    }
}

impl<P> Job<(Command, P, Empty, Empty)> {
    pub fn add_to_cgroup(self, cgroup_path: &Path) -> Result<Job<(Command, P, Empty, Initialized)>, Error> {
        let Self { id, cancel, status, handle, ..} = self;
        let (mut cmd, job_dir, _, _) = handle;
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
            handle: (cmd, job_dir, Empty, Initialized),
        })
    }
}

impl Job<(Command, &Path, Initialized, Initialized)> {
    pub fn spawn(self) -> Result<Job<Started>, Error> {
        let Self { id, cancel, status, handle, ..} = self;
        let (mut cmd, job_dir, ..) = handle;
        
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

        tokio::spawn(async move {
            let wout = File::create(outfile).await?;
            let werr = File::create(errfile).await?;
            let mut wout = BufWriter::new(wout);
            let mut werr = BufWriter::new(werr);

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
                                let mut status = status_clone.write().unwrap();
                                *status = JobStatus::Exit(outcome.raw_os_error().unwrap());
                            },
                        }

                        break;
                    }
                }
            }

            Ok::<(), std::io::Error>(())
        });


        Ok(Job {
            id,
            cancel,
            status,
            handle: Started,
        })
    }
}

impl<P> Job<(Command, P, Empty, Initialized)> {
    pub fn set_ownership(self, uid: u32, gid: u32) -> Job<(Command, P, Initialized, Initialized)> {
        let Self { id, cancel, status, handle, ..} = self;
        let (mut cmd, job_dir, _, cgroup) = handle;
        
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
            handle: (cmd, job_dir, Initialized, cgroup),
        }
    }
}
