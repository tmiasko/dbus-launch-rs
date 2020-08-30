use crate::pipe::Pipe;
use crate::sys::{close_on_exec_from, execvpe, set_close_on_exec};
use std::ffi::{CString, OsStr};
use std::io::{Error, ErrorKind, Read, Result, Write};
use std::os::raw::{c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::FromRawFd;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use std::ptr;
use std::time::Duration;

#[derive(Debug)]
pub(crate) struct Process {
    pid: libc::pid_t,
    exit_status: Option<ExitStatus>,
}

impl Process {
    /// Spawns a new dbus-daemon process using specified config file.
    pub(crate) fn spawn_dbus_daemon(
        program: Option<&OsStr>,
        config: &Path,
    ) -> Result<(Self, String)> {
        let (mut r, w) = Pipe::new()?;

        let mut argv = CStringArray::new();
        argv.push(program.unwrap_or(OsStr::new("dbus-daemon")));
        argv.push("--nofork");
        argv.push("--config-file");
        argv.push(config);
        argv.push("--print-address=3");
        let env = ptr::null();
        let process = spawn(argv.as_ptr(), env, &mut || {
            if w.as_raw_fd() != 3 && unsafe { libc::dup2(w.as_raw_fd(), 3) } == -1 {
                return Err(Error::last_os_error());
            }
            set_close_on_exec(3, false)
        })?;

        // Read the address from the pipe.
        drop(w);
        let mut address = String::new();
        r.read_to_string(&mut address)?;
        address = address.trim().to_string();

        if !address.is_empty() {
            Ok((process, address))
        } else {
            Err(Error::new(
                ErrorKind::Other,
                "dbus-daemon returned empty address",
            ))
        }
    }

    /// Spawns a new dbus-broker process using specified config file and listening socket.
    pub(crate) fn spawn_dbus_broker(
        program: Option<&OsStr>,
        config: &Path,
        socket: c_int,
    ) -> Result<Self> {
        let mut argv = CStringArray::new();
        argv.push(program.unwrap_or(OsStr::new("dbus-broker-launch")));
        argv.push("--config-file");
        argv.push(config);

        let mut env = CStringArray::new();
        for (mut var, val) in std::env::vars_os() {
            if var == "LISTEN_PID" || var == "LISTEN_FDS" {
                // Ignore. They have to be overwritten later anyway.
                continue;
            }
            var.push("=");
            var.push(val);
            env.push(var);
        }
        env.push("LISTEN_FDS=1");
        let mut listen_pid = [0u8; 30];
        env.push_ptr(listen_pid.as_ptr().cast());

        spawn(argv.as_ptr(), env.as_ptr(), &mut || {
            if socket != 3 && unsafe { libc::dup2(socket, 3) } == -1 {
                return Err(Error::last_os_error());
            }
            set_close_on_exec(3, false)?;
            write!(&mut listen_pid[..], "LISTEN_PID={}\0", unsafe {
                libc::getpid()
            })
        })
    }

    pub(crate) fn pid(&self) -> libc::pid_t {
        self.pid
    }

    pub(crate) fn kill(&mut self, signal: c_int) -> Result<()> {
        if self.exit_status.is_some() {
            return Ok(());
        }

        if unsafe { libc::kill(self.pid, signal) } == -1 {
            Err(Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub(crate) fn wait(&mut self) -> Result<ExitStatus> {
        if let Some(status) = self.exit_status {
            return Ok(status);
        };

        let mut status = 0;
        if unsafe { libc::waitpid(self.pid, &mut status, 0) } == -1 {
            Err(Error::last_os_error())
        } else {
            let status = ExitStatus::from_raw(status);
            self.exit_status = Some(status);
            Ok(status)
        }
    }

    pub(crate) fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        if let Some(status) = self.exit_status {
            return Ok(Some(status));
        };

        let mut status = 0;
        let ret = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
        if ret == -1 {
            Err(Error::last_os_error())
        } else if ret == 0 {
            Ok(None)
        } else {
            let status = ExitStatus::from_raw(status);
            self.exit_status = Some(status);
            Ok(Some(status))
        }
    }

    pub(crate) fn try_wait_timeout(
        &mut self,
        mut timeout: Duration,
    ) -> Result<Option<ExitStatus>> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(Some(status));
            }
            if let Some(left) = timeout.checked_sub(Duration::from_secs(1)) {
                timeout = left;
                std::thread::sleep(Duration::from_secs(1));
            } else {
                std::thread::sleep(timeout);
                return self.try_wait();
            }
        }
    }
}

fn spawn(
    argv: *const *const c_char,
    env: *const *const c_char,
    pre_exec: &mut dyn FnMut() -> Result<()>,
) -> Result<Process> {
    let (mut r, mut w) = Pipe::new()?;

    if w.as_raw_fd() <= 3 {
        // Avoid conflict with listen fd / print-address fd.
        let fd =
            unsafe { libc::fcntl(w.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 4 as c_int) };
        if fd == -1 {
            return Err(Error::last_os_error());
        }
        assert!(fd > 3);
        w = unsafe { Pipe::from_raw_fd(fd) };
    }

    let pid = unsafe { libc::fork() };
    if pid == -1 {
        Err(Error::last_os_error())
    } else if pid == 0 {
        // Child process
        let error = try_exec(argv, env, pre_exec);
        let error = error.raw_os_error().unwrap_or(libc::EINVAL) as u32;
        let error = error.to_ne_bytes();
        let _ = w.write_all(&error);
        unsafe { libc::_exit(1) };
    } else {
        // Parent process
        let mut p = Process {
            pid,
            exit_status: None,
        };
        drop(w);
        let mut error = [0u8; 4];
        match r.read_exact(&mut error[..]) {
            Ok(()) => {
                let error = i32::from_ne_bytes(error);
                let _ = p.wait();
                Err(Error::from_raw_os_error(error))
            }
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => Ok(p),
            Err(_) => unreachable!(),
        }
    }
}

fn try_exec(
    argv: *const *const c_char,
    env: *const *const c_char,
    pre_exec: &mut dyn FnMut() -> Result<()>,
) -> Error {
    for &s in &[
        libc::SIGCHLD,
        libc::SIGINT,
        libc::SIGTERM,
        libc::SIGHUP,
        libc::SIGPIPE,
    ] {
        if unsafe { libc::signal(s, libc::SIG_DFL) } == libc::SIG_ERR {
            return Error::last_os_error();
        }
    }

    if let Err(err) = close_on_exec_from(3) {
        return err;
    }

    if let Err(err) = pre_exec() {
        return err;
    }

    if env.is_null() {
        unsafe { libc::execvp(*argv, argv) };
    } else {
        unsafe { execvpe(*argv, argv, env) };
    }

    Error::last_os_error()
}

struct CStringArray {
    owned: Vec<CString>,
    array: Vec<*const c_char>,
}

impl CStringArray {
    fn new() -> Self {
        let mut this = CStringArray {
            owned: Vec::new(),
            array: Vec::new(),
        };
        this.array.push(ptr::null());
        this
    }

    fn push<S>(&mut self, s: S)
    where
        S: AsRef<OsStr>,
    {
        let s = CString::new(s.as_ref().as_bytes()).unwrap();
        let p = s.as_ptr();
        self.owned.push(s);

        self.array.push(ptr::null());
        let last = self.array.len() - 2;
        self.array[last] = p;
    }

    fn push_ptr(&mut self, ptr: *const c_char) {
        self.array.push(ptr::null());
        let last = self.array.len() - 2;
        self.array[last] = ptr;
    }

    fn as_ptr(&self) -> *const *const c_char {
        self.array.as_ptr()
    }
}
