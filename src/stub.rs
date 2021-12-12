/// Used for stubbing libc functionality;
use nix::libc::{c_char, passwd};

pub fn setuid(_: i32) -> i32 { 0 }
pub fn setgid(_: i32) -> i32 { 0 }
pub fn useradd(_: &str) -> Result<(), crate::controller::Error> { Ok(()) }

pub unsafe fn getpwnam(_: *const c_char) -> *mut passwd {
    let empty = std::ptr::null::<i8>() as *mut i8;
    println!("Called getpwnam");
    let pwd = passwd {
        pw_name: empty,
        pw_passwd: empty,
        pw_uid: 123,
        pw_gid: 456,
        pw_change: 12345,
        pw_class: empty,
        pw_gecos: empty,
        pw_dir: empty,
        pw_shell: empty,
        pw_expire: 12345,
    };

    &pwd as *const _ as *mut _

}

