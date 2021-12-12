use log::error;

use nix::{
    libc::{setgid, setuid},
    unistd::getpid,
};

use std::{
    collections::HashMap,
    os::unix::process::ExitStatusExt,
    path::Path,
    process::Stdio,
    sync::{Arc, RwLock},
};

use tokio::{
    fs::File,
    io::{AsyncWriteExt, BufReader, BufWriter},
    process::Command,
    sync::Notify,
};

use uuid::Uuid;

use crate::{stack_string, PlsError, BASE_CG_PATH, BASE_PATH};

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

pub struct Job {
    id: Uuid,
    cancel: Arc<Notify>,
    status: Arc<RwLock<JobStatus>>,
}

impl Default for Job {
    fn default() -> Self {
        let id = Uuid::new_v4();
        let cancel = Arc::new(Notify::new());
        let status = Arc::new(RwLock::new(JobStatus::default()));
        Self { id, cancel, status }
    }
}

impl Job {
    pub fn new() -> Self {
        let id = Uuid::new_v4();
        let cancel = Arc::new(Notify::new());
        let status = Arc::new(RwLock::new(JobStatus::default()));

        Self { id, cancel, status }
    }
}

pub struct Controller<'c, J, IN> {
    client: &'c str,
    client_uid: u32,
    client_gid: u32,
    jobs: HashMap<Uuid, J>,
    inotify: IN,
}

impl<'c, IN> Controller<'c, Job, IN> {
    // Resolve gid/pid;
    // Create user for client if not exists
    // Create dir for jobs at base path + client name
    // Create cgroup at cg_base_path + client name
    // PlsError Or specific Controller errors?
    pub fn new(client: &'c str, inotify: IN) -> Result<Self, PlsError> {
        let client_uid = 123;
        let client_gid = 456;
        Ok(Self {
            client,
            client_uid,
            client_gid,
            jobs: HashMap::new(),
            inotify,
        })
    }

    // Create cgroup for job, "base path + client name + uuid", apply constraints if any
    pub async fn start(&mut self, _job_req: ()) -> Result<Uuid, PlsError> {
        let job = Job::new();
        let job_id = job
            .id
            .to_simple()
            .encode_lower(&mut Uuid::encode_buffer())
            .to_owned();

        let job_dir = Path::new(BASE_PATH).join(self.client).join(&job_id);
        tokio::fs::create_dir_all(&job_dir).await?;

        // Enable controllers after creating
        let cgroup_path = Path::new(BASE_CG_PATH).join(self.client).join(&job_id);
        tokio::fs::create_dir_all(&cgroup_path).await?;

        let cgroup_procs: stack_string::String<256> = cgroup_path
            .join("cgroup.procs")
            // BASE_PATH is &str,
            // client is &str,
            // job_id is &str,
            // all &str's are valid utf8
            .to_str()
            .expect("Panic on valid utf8")
            .try_into()?;

        // actual command from job req
        let mut cmd = Command::new("node");
        // actual args from job req
        cmd.args(&["index.js"])
            .current_dir(&job_dir)
            .stdout(Stdio::piped())
            .stdin(Stdio::piped());

        // Safety:
        // All calls inside closure are async-signal-safe
        unsafe {
            cmd.pre_exec(move || {
                let pid: stack_string::String<11> = getpid().as_raw().into();
                std::fs::write(cgroup_procs, pid)?;
                Ok(())
            });

            let uid = self.client_uid;
            let gid = self.client_gid;

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

        let mut child = cmd.spawn()?;

        // Unwrap: Command is instantiated within the scope of this function
        // therefore out/error must be present
        let out = child.stdout.take().expect("Child stdout is missing");
        let err = child.stderr.take().expect("Child stderr is missing");

        let mut out = BufReader::new(out);
        let mut err = BufReader::new(err);

        let cancel = job.cancel.clone();
        let status = job.status.clone();

        tokio::spawn(async move {
            let wout = File::create(&job_dir.join("out")).await?;
            let werr = File::create(&job_dir.join("err")).await?;
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
                    _ = cancel.notified() => {
                        wout.flush().await?;
                        werr.flush().await?;
                        child.kill().await?;
                        // Can error only if RwLock is poisoned
                        // which cannot happen b/c it is the fist and only
                        // time this lock is used for writing;
                        let mut status = status.write().unwrap();
                        *status = JobStatus::Signal(9);
                        break;
                    }
                    outcome = child.wait() => {
                        wout.flush().await?;
                        werr.flush().await?;
                        match outcome {
                            Ok(outcome) => {
                                if let Some(code) = outcome.code() {
                                    let mut status = status.write().unwrap();
                                    *status = JobStatus::Exit(code);
                                } else {
                                    let signal = outcome.signal()
                                        .expect("Neither exit code nor signal");
                                    let mut status = status.write().unwrap();
                                    *status = JobStatus::Signal(signal);
                                }
                            },

                            Err(outcome) => {
                                let mut status = status.write().unwrap();
                                *status = JobStatus::Exit(outcome.raw_os_error().unwrap());
                            },
                        }

                        break;
                    }
                }
            }

            Ok::<(), std::io::Error>(())
        });

        let job_id = job.id;
        self.jobs.insert(job.id, job);

        Ok(job_id)
    }
}
