use std::{path::Path, str::from_utf8_unchecked};
use thiserror::Error;

/// As per [fork(2)](https://man7.org/linux/man-pages/man2/fork.2.html) only async-signal-safe
/// functions should be called after `fork` until `execve` is called.
/// To ensure no allocations are done in `Command::pre_exec` stack-backed string is used for:
/// - Transferring ownership of constructed cgroup Path to closure
/// - Writing Pid of the child process as String to cgroup.procs
#[derive(Copy, Clone, Debug)]
pub struct String<const N: usize> {
    buf: [u8; N],
    len: usize,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Supplied data (len:{0}) won't fit into buffer (len:{1})")]
    InvalidBufSize(usize, usize),
}

impl<const N: usize> core::fmt::Display for String<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const N: usize> String<N> {
    fn new() -> Self {
        Self {
            buf: [0u8; N],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        if self.len == 0 {
            ""
        } else {
            // Safety:
            // Can only be created from &str
            // Or i32 which is serialized as utf8
            unsafe { from_utf8_unchecked(&self.buf[..self.len]) }
        }
    }
}

impl From<i32> for String<11> {
    fn from(value: i32) -> Self {
        let mut s_string = Self::new();
        let len = serialize_i32(value, &mut s_string.buf)
            .expect("Failed to fit utf8 representation of i32 into 11 bytes");

        s_string.len = len;
        s_string
    }
}

impl<const N: usize> TryFrom<&str> for String<N> {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let len = value.len();
        if len > N {
            return Err(Error::InvalidBufSize(len, N));
        }
        let mut s_string = Self::new();
        s_string.buf[..len].copy_from_slice(&value.as_bytes()[..len]);
        s_string.len = len;

        Ok(s_string)
    }
}

impl<const N: usize> AsRef<[u8]> for String<N> {
    fn as_ref(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl<const N: usize> AsRef<str> for String<N> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<const N: usize> AsRef<Path> for String<N> {
    fn as_ref(&self) -> &Path {
        Path::new(self.as_str())
    }
}

pub fn serialize_i32(num: i32, buf: &mut [u8]) -> Result<usize, Error> {
    // i32::MAX_VALUE fits in 10 digits + 1 for sign
    if buf.len() < 11 {
        return Err(Error::InvalidBufSize(11, buf.len()));
    }

    let mut cursor = 0;
    let mut abs = num.abs();

    if abs == 0 {
        buf[cursor] = b'0';
        return Ok(1);
    }

    while abs > 0 {
        let digit = (abs % 10) as u8;
        abs /= 10;
        buf[cursor] = b'0' + digit;
        cursor += 1;
    }

    if num.is_negative() {
        buf[cursor] = b'-';
        cursor += 1;
    }

    buf[..cursor].reverse();

    Ok(cursor)
}

#[cfg(test)]
mod tests {

    #[test]
    fn given_negative_i32_starts_with_sign() {
        let int = -7123456;
        let s_string: super::String<11> = int.into();
        assert_eq!(s_string.as_str(), "-7123456");
    }

    #[test]
    fn given_positive_i32_does_not_include_sign() {
        let int = 123456;
        let s_string: super::String<11> = int.into();
        assert_eq!(s_string.as_str(), "123456");
    }

    #[test]
    fn given_negative_zero_serializes_zero() {
        let int = -0;
        let s_string: super::String<11> = int.into();
        assert_eq!(s_string.as_str(), "0");
    }

    #[test]
    fn given_positive_zero_serializes_zero() {
        let int = 0;
        let s_string: super::String<11> = int.into();
        assert_eq!(s_string.as_str(), "0");
    }

    #[test]
    fn given_str_wont_fit_fails_to_create() {
        let input = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.";
        let outcome: Result<super::String<1>, super::Error> = input.try_into();
        let err_string = format!(
            "Supplied data (len:{}) won't fit into buffer (len:1)",
            input.len()
        );
        assert!(outcome.is_err());
        assert_eq!(outcome.unwrap_err().to_string(), err_string);
    }
}
