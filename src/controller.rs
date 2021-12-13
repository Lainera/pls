use log::error;
use thiserror::Error;
use crate::runner::JobRequest;
use crate::job::{Job, Started};

#[cfg(target_os = "linux")]
use nix::libc::getpwnam;

#[cfg(target_os = "linux")]
// Synchronously adds user during setup phase to avoid
// dealing with boxing of recursive future
fn useradd<'c>(client: &'c str) -> Result<(), Error> {
    let args = vec!["-s", "/sbin/nologin", "-U", client];
    std::process::Command::new("useradd")
        .args(&args)
        .status()?;
    
    Ok(())
}

#[cfg(not(target_os = "linux"))]
use crate::stub::{getpwnam, useradd};

use std::{
    collections::HashMap,
    path::Path,
    ffi::{CString, NulError},
};

use tokio::fs::create_dir_all;

use uuid::Uuid;

use crate::{job, BASE_CG_PATH, BASE_PATH};


// TODO: 
// Add cleanup task which removes empty cgroups when Job exits
#[derive(Debug)]
pub struct Controller<'c, J, IN> {
    client: &'c str,
    client_uid: u32,
    client_gid: u32,
    jobs: HashMap<Uuid, J>,
    inotify: IN,
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
}

impl<'c, IN> Controller<'c, Job<Started>, IN> {
    fn ensure_uid_gid(client: &'c str) -> Result<(u32, u32), Error> {
        let cstr = CString::new(client.as_bytes())?;
        
        // Safety: ptr is null checked, and if user does not exist, user is created;
        let passwd = unsafe { getpwnam(cstr.as_ptr()) };
        if passwd.is_null() {
            useradd(client)?;
            return Self::ensure_uid_gid(client)
        } 

        let (uid, gid) = unsafe {
            ((*passwd).pw_uid, (*passwd).pw_gid)
        };

        Ok((uid, gid))
    }

    pub fn new(client: &'c str, inotify: IN) -> Result<Controller<'c, Job<Started>, IN>, Error> {
        let (client_uid, client_gid) = Self::ensure_uid_gid(client)?;
        Ok(Self {
            client,
            client_uid,
            client_gid,
            jobs: HashMap::new(),
            inotify,
        })
    }

    // Create cgroup for job, "base path + client name + uuid", apply constraints if any
    //
    // Cgroups stuff should be handled by static namespace
    // file names should be constants on it;
    // cgroup::enable(&path, ControllersMask) -> Result
    // cgroup::disable(&path, ControllersMask) -> Result
    // cgroup::cpu_weight(&path, opts) -> Result
    // cgroup::mem_high() .. and so on;
    pub async fn start(&mut self, job_request: JobRequest) -> Result<Uuid, Error> {
        let job = Job::default();
        let job_id = job
            .id()
            .to_simple()
            .encode_lower(&mut Uuid::encode_buffer())
            .to_owned();

        let job_dir = Path::new(BASE_PATH).join(self.client).join(&job_id);
        create_dir_all(&job_dir).await?;

        // Enable controllers after creating
        let cgroup_path = Path::new(BASE_CG_PATH).join(self.client).join(&job_id);
        create_dir_all(&cgroup_path).await?;

        let job = job
            .add_command(&job_request)
            .add_to_cgroup(&cgroup_path)?
            .set_ownership(self.client_uid, self.client_gid)
            .set_job_dir(&job_dir)
            .spawn()?;

        let job_id: Uuid = job.id().to_owned();
        self.jobs.insert(job_id, job);

        Ok(job_id)
    }
}
