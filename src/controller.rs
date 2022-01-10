use crate::job::{Job, Started};
use crate::runner::{self, JobRequest, LogMessage};
use log::error;
use thiserror::Error;
use tokio::{
    io::AsyncReadExt,
    sync::{
        mpsc::{self, Receiver},
        watch,
    },
};

use nix::libc::getpwnam;
// Synchronously adds user during setup phase to avoid
// dealing with boxing of recursive future
fn useradd<'c>(client: &'c str) -> Result<(), Error> {
    let args = vec!["-s", "/sbin/nologin", "-U", client];
    std::process::Command::new("useradd").args(&args).status()?;

    Ok(())
}

use std::{
    collections::HashMap,
    ffi::{CString, NulError},
    path::Path,
};

use tokio::fs::{create_dir_all, File};

use uuid::Uuid;

use crate::{cgroup, job, BASE_CG_PATH, BASE_PATH};

pub enum Fd {
    Out,
    Err,
}

impl From<Fd> for i32 {
    fn from(fd: Fd) -> Self {
        match fd {
            Fd::Out => 0,
            Fd::Err => 1,
        }
    }
}

#[derive(Debug)]
pub struct Controller<'c, J> {
    client: &'c str,
    client_uid: u32,
    client_gid: u32,
    jobs: HashMap<Uuid, J>,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid client name")]
    CStringError(#[from] NulError),
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error("Failed to cast")]
    Conversion(#[from] std::num::TryFromIntError),
    #[error(transparent)]
    JobError(#[from] job::Error),
    #[error("Failed to find job({0})")]
    JobNotFound(Uuid),
    #[error(transparent)]
    Cgroup(#[from] cgroup::Error),
}

impl<'c> Controller<'c, Job<Started>> {
    pub async fn new(client: &'c str) -> Result<Controller<'c, Job<Started>>, Error> {
        let (client_uid, client_gid) = Self::ensure_uid_gid(client)?;
        let cgroup_dir = Path::new(BASE_CG_PATH).join(client);
        create_dir_all(&cgroup_dir).await?;
        cgroup::enable_subtree(&cgroup_dir, cgroup::Controller::all()).await?;

        Ok(Self {
            client,
            client_uid,
            client_gid,
            jobs: HashMap::new(),
        })
    }

    pub async fn start(&mut self, job_request: JobRequest) -> Result<Uuid, Error> {
        let job = Job::default();
        let job_id = job
            .id()
            .to_simple()
            .encode_lower(&mut Uuid::encode_buffer())
            .to_owned();

        let job_dir = Path::new(BASE_PATH).join(self.client).join(&job_id);
        create_dir_all(&job_dir).await?;

        let cgroup_dir = Path::new(BASE_CG_PATH).join(self.client).join(&job_id);
        create_dir_all(&cgroup_dir).await?;
        cgroup::set_cpu_control(&cgroup_dir, &job_request).await?;
        cgroup::set_mem_control(&cgroup_dir, &job_request).await?;
        cgroup::set_io_control(&cgroup_dir, &job_request).await?;

        let job = job
            .add_command(&job_request)
            .add_to_cgroup(cgroup_dir)?
            .set_ownership(self.client_uid, self.client_gid)
            .set_job_dir(job_dir)
            .spawn()?;

        let job_id: Uuid = job.id().to_owned();
        self.jobs.insert(job_id, job);

        Ok(job_id)
    }

    pub async fn status(&self, job_id: Uuid) -> Result<runner::JobStatus, Error> {
        let job = self.jobs.get(&job_id).ok_or(Error::JobNotFound(job_id))?;
        Ok(job.status())
    }

    pub async fn stop(&mut self, job_id: Uuid) -> runner::Ack {
        if let Some(job) = self.jobs.get(&job_id) {
            job.cancel();
        }

        runner::Ack {}
    }

    pub async fn output(&self, job_id: Uuid) -> Result<Receiver<Result<LogMessage, Error>>, Error> {
        let job = self.jobs.get(&job_id).ok_or(Error::JobNotFound(job_id))?;
        let job_dir = job.job_dir();

        let (tx, rx) = mpsc::channel(20);

        if job.is_complete() {
            Self::read_file(Fd::Out, job_dir, tx.clone()).await;
            Self::read_file(Fd::Err, job_dir, tx).await;

            Ok(rx)
        } else {
            let completion = job.subscribe();
            Self::watch_file(Fd::Out, job_dir, completion.clone(), tx.clone()).await;
            Self::watch_file(Fd::Err, job_dir, completion, tx).await;

            Ok(rx)
        }
    }

    async fn watch_file(
        fd: Fd,
        job_dir: &Path,
        mut completion: watch::Receiver<bool>,
        tx: mpsc::Sender<Result<LogMessage, Error>>,
    ) {
        let file_path = match fd {
            Fd::Out => job_dir.join("out"),
            Fd::Err => job_dir.join("err"),
        };

        tokio::spawn(async move {
            let fd: i32 = fd.into();
            let mut buf = [0u8; 512];
            let mut file = File::open(file_path).await?;
            let mut done = false;

            loop {
                tokio::select! {
                    _ = completion.changed() => {
                        // exit on next EOF;
                        done = true;
                    }
                   outcome = file.read(&mut buf) => {
                       match outcome {
                           Ok(bytes_read) if bytes_read > 0 => {
                               let msg = LogMessage { fd, output: buf[..bytes_read].to_vec() };
                               if let Err(err) = tx.send(Ok(msg)).await {
                                   error!("Failed to send read logs({})", err);
                                   break;
                               }
                           }
                           Ok(bytes_read) if bytes_read == 0 && done => break,
                           Err(err) => {
                               error!("Error reading from log file: {}", err);
                               break;
                           },
                           _ => (),
                       }
                   }

                }
            }

            Ok::<(), std::io::Error>(())
        });
    }

    async fn read_file(fd: Fd, job_dir: &Path, tx: mpsc::Sender<Result<LogMessage, Error>>) {
        let file_path = match fd {
            Fd::Out => job_dir.join("out"),
            Fd::Err => job_dir.join("err"),
        };

        tokio::spawn(async move {
            let fd: i32 = fd.into();
            let mut file = File::open(file_path).await?;
            let mut buffer = [0u8; 512];

            while let Ok(bytes_read) = file.read(&mut buffer).await {
                let msg = LogMessage {
                    fd,
                    output: buffer[..bytes_read].to_vec(),
                };
                if let Err(err) = tx.send(Ok(msg)).await {
                    error!("Failed to send output message: {}", err);
                    break;
                }
            }

            Ok::<(), std::io::Error>(())
        });
    }

    fn ensure_uid_gid(client: &'c str) -> Result<(u32, u32), Error> {
        let cstr = CString::new(client.as_bytes())?;

        // Safety: ptr is null checked, and if user does not exist, user is created;
        let passwd = unsafe { getpwnam(cstr.as_ptr()) };
        if passwd.is_null() {
            useradd(client)?;
            return Self::ensure_uid_gid(client);
        }

        let (uid, gid) = unsafe { ((*passwd).pw_uid, (*passwd).pw_gid) };

        Ok((uid, gid))
    }
}
