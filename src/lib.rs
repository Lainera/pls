#![allow(dead_code)]

pub mod runner {
    tonic::include_proto!("runner");
}

use thiserror::Error;
pub mod controller;
pub mod job;
pub mod stack_string;

pub mod cgroup {
    pub const PROC_FILE: &str = "cgroup.procs";
}

#[cfg(not(target_os = "linux"))]
pub(crate) mod stub;

#[derive(Error, Debug)]
pub enum PlsError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    CTError(#[from] controller::Error),
}

#[derive(Debug)]
pub struct Empty;

const BASE_PATH: &str = "/tmp/pls/clients";
#[cfg(target_os = "linux")]
const BASE_CG_PATH: &str = "/sys/fs/cgroup";

#[cfg(not(target_os = "linux"))]
const BASE_CG_PATH: &str = "/tmp/fs/cgroup";

mod service {
    use std::collections::HashMap;

    use crate::controller::Controller;

    pub struct Service<'c, J> {
        controllers: HashMap<&'c str, &'c Controller<'c, J>>,
    }
}
