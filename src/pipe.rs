use libc::{self, c_int, c_void};
use std::io::{Error, Read, Result, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

pub(crate) struct Pipe {
    fd: c_int,
}

impl Pipe {
    #[cfg(not(target_os = "macos"))]
    pub fn new() -> Result<(Pipe, Pipe)> {
        let mut fds = [0; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } == -1 {
            return Err(Error::last_os_error());
        }
        let r = unsafe { Pipe::from_raw_fd(fds[0]) };
        let w = unsafe { Pipe::from_raw_fd(fds[1]) };
        Ok((r, w))
    }

    #[cfg(target_os = "macos")]
    pub fn new() -> Result<(Pipe, Pipe)> {
        use crate::sys::set_close_on_exec;

        let mut fds = [0; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
            return Err(Error::last_os_error());
        }
        let r = unsafe { Pipe::from_raw_fd(fds[0]) };
        let w = unsafe { Pipe::from_raw_fd(fds[1]) };
        set_close_on_exec(r.as_raw_fd(), true)?;
        set_close_on_exec(w.as_raw_fd(), true)?;
        Ok((r, w))
    }
}

impl FromRawFd for Pipe {
    unsafe fn from_raw_fd(fd: RawFd) -> Pipe {
        Pipe { fd }
    }
}

impl IntoRawFd for Pipe {
    fn into_raw_fd(self) -> RawFd {
        let Pipe { fd } = self;
        fd
    }
}

impl AsRawFd for Pipe {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n =
            unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
        if n == -1 {
            Err(Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }
}

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let n = unsafe { libc::write(self.fd, buf.as_ptr() as *mut c_void, buf.len()) };
        if n == -1 {
            Err(Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}
