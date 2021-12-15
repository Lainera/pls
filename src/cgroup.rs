use core::fmt;
use std::path::{Path, PathBuf};
use thiserror::Error;
use crate::runner::JobRequest;

pub const PROC_FILE: &str = "cgroup.procs";
pub const ENABLED_CONTROLLERS: &str = "cgroup.controllers";
pub const SUBTREE_CONTROL: &str = "cgroup.subtree_control";
pub const CPU_WEIGHT: &str = "cpu.weight";
pub const MEM_HIGH: &str = "memory.high";
pub const MEM_MAX: &str = "memory.max";
pub const IO_MAX: &str = "io.max";

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    IO(#[from] std::io::Error),

    #[error("Some controllers at {0} are not enabled")]
    NotEnabled(PathBuf), 

    #[error("{0} unknown controller name")]
    UnknownController(String),

    #[error("{0} is invalid, valid range is [1, 10000]")]
    InvalidCpuWeight(u32),
}

#[derive(Debug, PartialEq)]
pub enum Controller {
    Cpu,
    Memory,
    Io,
}

impl Controller {
    /// Return list of all supported controllers 
    pub const fn all() -> &'static [Controller] {
        &[Controller::Cpu, Controller::Memory, Controller::Io]
    } 
}


impl From<&Controller> for &str {
    fn from(value: &Controller) -> Self {
        match value {
            Controller::Cpu => "cpu",
            Controller::Memory => "memory",
            Controller::Io => "io",
        }
    }
}

impl TryFrom<&str> for Controller {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "memory" => Ok(Controller::Memory),
            "cpu" => Ok(Controller::Cpu),
            "io" => Ok(Controller::Io),
            _ => Err(Error::UnknownController(value.to_string())),
        }
    }
}

impl fmt::Display for Controller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.into())
    }
}

impl PartialEq<str> for Controller {
    fn eq(&self, other: &str) -> bool {
        self == other
    }
}

pub async fn enable_subtree(cgroup_dir: &Path, controllers: &[Controller]) -> Result<(), Error> {
    is_enabled(cgroup_dir, controllers).await?;
    enable_subtree_unchecked(cgroup_dir, controllers).await
}

pub async fn set_cpu_control(cgroup_dir: &Path, job_req: &JobRequest) -> Result<(), Error> {
    if let Some(cpu_control) = &job_req.cpu_control {
        let weight = cpu_control.cpu_weight;
        if weight == 0 || weight > 10000 {
            return Err(Error::InvalidCpuWeight(weight))
        }

        tokio::fs::write(cgroup_dir.join(CPU_WEIGHT),  weight.to_string()).await?;
    }

    Ok(())
}

pub async fn set_mem_control(cgroup_dir: &Path, job_req: &JobRequest) -> Result<(), Error> {
    if let Some(memory_control) = &job_req.mem_control {
        tokio::fs::write(cgroup_dir.join(MEM_HIGH), memory_control.mem_high.to_string()).await?;
        tokio::fs::write(cgroup_dir.join(MEM_MAX), memory_control.mem_max.to_string()).await?;
    }

    Ok(())
}

pub async fn set_io_control(cgroup_dir: &Path, job_req: &JobRequest) -> Result<(), Error> {
    if let Some(io_control) = &job_req.io_control {
        let contents = format!(
            "{}:{} rbps={} wbps={}", 
            io_control.minor, io_control.major, io_control.rbps_max, io_control.wbps_max
        );
        tokio::fs::write(cgroup_dir.join(IO_MAX), contents).await?;
    }

    Ok(())
}

pub async fn enable_subtree_unchecked(cgroup_dir: &Path, controllers: &[Controller]) -> Result<(), Error> {
    let contents = prepend_with(controllers, '+');
    tokio::fs::write(cgroup_dir.join(SUBTREE_CONTROL), contents).await?;
    Ok(())
}

async fn is_enabled(cgroup_dir: &Path, controllers: &[Controller]) -> Result<(), Error> {
    let enabled = cgroup_dir.join(ENABLED_CONTROLLERS);
    let enabled = tokio::fs::read_to_string(enabled).await?; 
    if is_subset(&enabled, controllers) {
        Ok(())
    } else {
        Err(Error::NotEnabled(cgroup_dir.to_owned()))
    }
}

fn prepend_with<'a, T>(list: &'a [T], joiner: char) -> String 
    where
        &'a str: From<&'a T>,
{
    list.iter()
        .map(|item| item.into())
        .enumerate()
        .fold(String::new(), |mut acc, (ix, item): (usize, &str)| {
            if ix > 0 {acc.push(' ');}
            acc.push(joiner);
            acc.push_str(item);
            acc
        })
}

fn is_subset(enabled: &str, controllers: &[Controller]) -> bool {
    let enabled = enabled.split(' ')
        .filter_map(|ctr| Controller::try_from(ctr).ok());
   
    controllers
        .iter()
        .all(|controller| enabled
             .clone()
             .any(|ctr| &ctr == controller)
        )
}

#[cfg(test)]
mod tests {
    use super::{Controller, prepend_with, is_subset};

    #[test]
    fn given_some_controllers_are_disabled_then_cant_enable() {
        let enabled = "memory cpu";
        let ctr = Controller::all();
        assert!(!is_subset(enabled, ctr));
    }

    #[test]
    fn given_required_controllers_are_present_then_can_enable() {
        let enabled = "memory cpu io";
        let ctr = Controller::all();
        assert!(is_subset(enabled, ctr));
    }

    #[test]
    fn given_unknown_controllers_when_known_are_present_then_can_enable() {
        let enabled = "memory cpu cpuset pids";
        let ctr = &[Controller::Memory, Controller::Cpu];
        assert!(is_subset(enabled, ctr));
    }
    
    #[test]
    fn given_list_of_controllers_generates_valid_output() {
        let ctr = Controller::all();
        let as_str = prepend_with(ctr, '+');
        assert_eq!(as_str, "+cpu +memory +io");
    }

    #[test]
    fn given_list_of_controllers_generates_valid_output_2() {
        let ctr = Controller::all();
        let as_str = prepend_with(ctr, '-');
        assert_eq!(as_str, "-cpu -memory -io");
    }

    #[test]
    fn given_empty_list_output_is_empty() {
        let ctr: &[Controller] = &[];
        let as_str = prepend_with(ctr, 'x');
        assert!(as_str.is_empty()); 
    }
}
