#![allow(dead_code)]

use thiserror::Error;
pub mod controller;
pub mod stack_string;

#[cfg(not(target_os = "linux"))]
pub(crate) mod stub;

#[derive(Error, Debug)]
pub enum PlsError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    STString(#[from] stack_string::Error),
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

    pub struct Service<'c, J, IN> {
        controllers: HashMap<&'c str, &'c Controller<'c, J, IN>>,
    }
}
