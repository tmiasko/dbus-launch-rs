//! Verifies that unrelated file descriptors are closed during exec, and
//! daemon does not inherit them.

use std::env;
use std::io::{Error, Result};
use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::thread::{self, JoinHandle};

fn main() {
    if env::var_os("CHILD").is_none() {
        parent();
    } else {
        child();
    }
}

fn parent() {
    let start_count = count_open_fds();

    // Launch daemon processes in parallel.
    launch_parallel(20);

    assert_eq!(start_count, count_open_fds());
}

fn launch_parallel(n: usize) {
    env::set_var("CHILD", "1");

    let threads: Vec<JoinHandle<()>> = (0..n)
        .map(|_| {
            let mut args = std::env::args();
            let argv0 = args.next().unwrap();
            thread::spawn(move || {
                // Open additional pipes without O_CLOEXEC
                // flag to make things more interesting.
                let _pipe = Pipe::new();
                let daemon = dbus_launch::Launcher::daemon()
                    .program(argv0.as_ref())
                    .launch()
                    .unwrap();
                assert_eq!(daemon.address(), "everything-ok");
            })
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }
}

fn child() {
    assert_eq!(4, count_open_fds());

    let mut fd = None;
    for arg in std::env::args() {
        if let Some(n) = arg.strip_prefix("--print-address=") {
            let n = n.parse::<c_int>().unwrap();
            assert_eq!(n, 3);
            fd = Some(n);
        }
    }

    let fd = fd.unwrap();
    unsafe {
        let address = "everything-ok";
        let n = libc::write(fd, address.as_ptr().cast(), address.len());
        assert_eq!(n, address.len() as isize);
        let n = libc::close(fd);
        assert_eq!(n, 0);
    };
}

struct Pipe {
    fds: [c_int; 2],
}

impl Pipe {
    fn new() -> Self {
        let mut fds = [0; 2];
        assert_ne!(unsafe { libc::pipe(fds.as_mut_ptr()) }, -1);
        Pipe { fds }
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        assert_eq!(unsafe { libc::close(self.fds[0]) }, 0);
        assert_eq!(unsafe { libc::close(self.fds[1]) }, 0);
    }
}

/// Returns a value one greater than the maximum file descriptor number.
fn open_fd_limit() -> Result<c_int> {
    unsafe {
        let mut limit = MaybeUninit::uninit();
        if libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) == -1 {
            Err(Error::last_os_error())
        } else {
            Ok(limit.assume_init().rlim_cur as c_int)
        }
    }
}

/// Returns the number of opened file descriptors.
fn count_open_fds() -> usize {
    let mut open = 0;
    let limit = open_fd_limit().expect("failed to determine fd limit");
    for fd in 0..limit {
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1 {
            open += 1;
        }
    }
    open
}
