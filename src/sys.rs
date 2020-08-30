use libc::{self, c_char, c_int};
use std::io::{Error, Result};
use std::mem::MaybeUninit;

/// Sets close on exec flag on given file descriptor.
pub(crate) fn set_close_on_exec(fd: c_int, close_on_exec: bool) -> Result<()> {
    let old = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if old == -1 {
        return Err(Error::last_os_error());
    }
    let new = if close_on_exec {
        old | libc::FD_CLOEXEC
    } else {
        old & !libc::FD_CLOEXEC
    };
    if old != new && unsafe { libc::fcntl(fd, libc::F_SETFD, new) } == -1 {
        return Err(Error::last_os_error());
    }
    Ok(())
}

/// Sets close on exec flag on all file descriptors >= min.
// poll on Darwin doesn't set POLLNVAL for closed fds.
#[cfg(not(target_os = "macos"))]
pub(crate) fn close_on_exec_from(min: c_int) -> Result<()> {
    let mut pfds = [libc::pollfd {
        fd: 0,
        events: 0,
        revents: 0,
    }; 512];

    let limit = get_fd_limit()?;
    let mut i = min;
    while i < limit {
        let n = (pfds.len() as c_int).min(limit - i);
        let pfds = &mut pfds[..n as usize];

        for (pfd, fd) in pfds.iter_mut().zip(i..i + n) {
            pfd.fd = fd as c_int;
            pfd.events = 0;
            pfd.revents = 0;
        }

        if unsafe {
            libc::poll(
                pfds.as_mut_ptr() as *mut libc::pollfd,
                pfds.len() as libc::nfds_t,
                0,
            )
        } == -1
        {
            return Err(Error::last_os_error());
        }

        for pfd in pfds {
            if pfd.revents & libc::POLLNVAL == 0 {
                set_close_on_exec(pfd.fd, true)?;
            }
        }

        i += n;
    }

    Ok(())
}

/// Sets close on exec flag on all file descriptors >= min.
#[cfg(target_os = "macos")]
pub(crate) fn close_on_exec_from(min: c_int) -> Result<()> {
    for fd in min..get_fd_limit()? {
        if let Err(err) = set_close_on_exec(fd, true) {
            if err.raw_os_error() != Some(libc::EBADF) {
                return Err(err);
            }
        }
    }
    Ok(())
}

fn get_fd_limit() -> Result<c_int> {
    let mut limit = MaybeUninit::uninit();
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) } == -1 {
        return Err(Error::last_os_error());
    }
    Ok(unsafe { limit.assume_init().rlim_cur as c_int })
}

#[cfg(not(target_os = "macos"))]
pub(crate) unsafe fn execvpe(
    file: *const c_char,
    argv: *const *const c_char,
    env: *const *const c_char,
) -> c_int {
    libc::execvpe(file, argv, env)
}

#[cfg(target_os = "macos")]
pub(crate) unsafe fn execvpe(
    file: *const c_char,
    argv: *const *const c_char,
    env: *const *const c_char,
) -> c_int {
    extern "C" {
        fn _NSGetEnviron() -> *mut *const *const c_char;
    }
    *_NSGetEnviron() = env;
    libc::execvp(file, argv)
}
