#![allow(dead_code)]

use thiserror::Error;
pub mod controller;
pub mod stack_string;

#[derive(Error, Debug)]
pub enum PlsError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    STString(#[from] stack_string::Error),
}

pub struct Empty;
const BASE_PATH: &str = "/tmp/pls/clients";
const BASE_CG_PATH: &str = "/sys/fs/cgroup";

mod service {
    use std::collections::HashMap;

    use crate::controller::Controller;

    pub struct Service<'c, J, IN> {
        controllers: HashMap<&'c str, &'c Controller<'c, J, IN>>,
    }
}
