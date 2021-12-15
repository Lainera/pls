#![allow(dead_code)]

pub mod runner {
    tonic::include_proto!("runner");
}

use thiserror::Error;
pub mod cgroup;
pub mod controller;
pub mod job;
pub mod stack_string;

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
const BASE_CG_PATH: &str = "/sys/fs/cgroup";

mod service {
    use std::collections::HashMap;

    use crate::controller::Controller;

    pub struct Service<'c, J> {
        controllers: HashMap<&'c str, &'c Controller<'c, J>>,
    }
}
