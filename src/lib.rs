//! A D-Bus daemon launcher.
//!
//! A tool for starting an new isolated instance of a dbus-daemon or a
//! dbus-broker, with option to configure and start services using D-Bus
//! activation.
//!
//! Intended for the use in integration tests of D-Bus services and utilities.
//!
//! # Examples
//!
//! ## Launching a dbus-daemon process
//!
//! ```no_run
//! // Start the dbus-daemon.
//! let daemon = dbus_launch::Launcher::daemon()
//!     .launch()
//!     .expect("failed to launch dbus-daemon");
//!
//! // Use dbus-daemon by connecting to `daemon.address()`.
//!
//! // Stop the dbus-daemon process by dropping it.
//! drop(daemon);
//! ```
//!
//! ## Starting custom services using D-Bus activation
//!
//! ```no_run
//! use std::path::Path;
//!
//! let daemon = dbus_launch::Launcher::daemon()
//!     .service("com.example.Test", Path::new("/usr/lib/test-service"))
//!     .launch()
//!     .expect("failed to launch dbus-daemon");
//!
//! // Use com.example.Test service by connecting to `daemon.address()`.
//!
//! ```

use crate::process::Process;
use crate::xml::XmlWriter;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::unix::ffi::*;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod pipe;
mod process;
mod sys;
mod xml;

/// A D-Bus daemon launcher.
#[derive(Clone, Debug)]
pub struct Launcher {
    program: Option<OsString>,
    daemon_type: DaemonType,
    config: Config,
    services: Vec<Service>,
}

#[derive(Clone, Debug, Default)]
struct Config {
    bus_type: Option<BusType>,
    allow_anonymous: bool,
    listen: Vec<String>,
    auth: Vec<Auth>,
    service_dirs: Vec<PathBuf>,
}


/// A type of a D-Bus daemon.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum DaemonType {
    /// A dbus-daemon from the reference implementation.
    DBusDaemon,
    /// A dbus-broker.
    DBusBroker,
}

#[derive(Clone, Debug)]
struct Service {
    name: String,
    exec: PathBuf,
}

/// A running D-Bus daemon process.
///
/// The process is killed on drop.
#[derive(Debug)]
pub struct Daemon {
    address: String,
    tmp_dir: tempfile::TempDir,
    process: Process,
}

/// An authentication mechanism.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Auth {
    Anonymous,
    External,
    DBusCookieSha1,
}

/// A well-known message bus type.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum BusType {
    Session,
    System,
}

impl Launcher {
    /// Returns a new launcher for given type of D-Bus daemon.
    pub fn new(daemon_type: DaemonType) -> Launcher {
        Launcher {
            program: None,
            daemon_type,
            config: Config::default(),
            services: Vec::default(),
        }
    }

    /// Returns a new launcher for dbus-daemon.
    pub fn daemon() -> Launcher {
        Self::new(DaemonType::DBusDaemon)
    }

    /// Returns a new launcher for dbus-broker.
    pub fn broker() -> Launcher {
        Self::new(DaemonType::DBusBroker)
    }

    /// The well-known type of the message bus.
    pub fn bus_type(&mut self, bus_type: BusType) -> &mut Self {
        self.config.bus_type = Some(bus_type);
        self
    }

    /// Listen on an additional address.
    ///
    /// By default daemon will listen on a Unix domain socket in a temporary
    /// directory.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let mut launcher = dbus_launch::Launcher::daemon();
    /// launcher.listen("tcp:host=localhost");
    /// launcher.listen("unix:abstract=");
    /// ```
    pub fn listen(&mut self, listen: &str) -> &mut Self {
        self.config.listen.push(listen.to_owned());
        self
    }

    /// Authorize connections using anonymous mechanism.
    ///
    /// This option has no practical effect unless the anonymous mechanism is
    /// also enabled.
    pub fn allow_anonymous(&mut self) -> &mut Self {
        self.config.allow_anonymous = true;
        self
    }

    /// Allow authorization mechanism.
    ///
    /// By default all known mechanisms are allowed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let mut launcher = dbus_launch::Launcher::daemon();
    /// launcher.auth(dbus_launch::Auth::External);
    /// ```
    pub fn auth(&mut self, auth: Auth) -> &mut Self {
        self.config.auth.push(auth);
        self
    }

    /// Adds a directory to search for .service files.
    pub fn service_dir<P: AsRef<Path>>(&mut self, path: P) -> &mut Self {
        let path = path.as_ref().to_path_buf();
        self.config.service_dirs.push(path);
        self
    }

    /// Adds a service file with given name and executable path.
    pub fn service<P: AsRef<Path>>(&mut self, name: &str, exec: P) -> &mut Self {
        let name = name.to_string();
        let exec = exec.as_ref().to_path_buf();
        self.services.push(Service { name, exec });
        self
    }

    #[doc(hidden)]
    pub fn program(&mut self, program: &OsStr) -> &mut Self {
        self.program = Some(program.to_owned());
        self
    }

    /// Starts the dbus-daemon process.
    pub fn launch(&self) -> io::Result<Daemon> {
        let mut config = self.config.clone();

        // Create temporary dir for configuration files.
        let tmp_dir = tempfile::Builder::new()
            .prefix("dbus-daemon-rs-")
            .tempdir()?;

        if DaemonType::DBusDaemon == self.daemon_type && config.listen.is_empty() {
            // We use unix:dir instead of unix:tmpdir to avoid using abstract
            // sockets on Linux which are currently poorly supported in Rust
            // ecosystem.
            let path = escape_path(&tmp_dir.path());
            let address = format!("unix:dir={}", &path);
            config.listen.push(address);
        }

        // Write service files.
        if !self.services.is_empty() {
            config.service_dirs.push(tmp_dir.path().to_owned());
            for service in &self.services {
                let file = format!("{}.service", service.name);
                let path = tmp_dir.path().join(&file);
                let contents = format!(
                    "[D-BUS Service]\nName={}\nExec={}\n",
                    service.name,
                    service.exec.display()
                );
                fs::write(path, contents)?;
            }
        }

        // Write daemon config file.
        let config_file = tmp_dir.path().join("daemon.conf");
        fs::write(&config_file, config.to_xml().as_bytes())?;

        let program = self.program.as_deref();
        match self.daemon_type {
            DaemonType::DBusDaemon => {
                let (process, address) =
                    Process::spawn_dbus_daemon(program, &config_file)?;
                Ok(Daemon {
                    address,
                    tmp_dir,
                    process,
                })
            }
            DaemonType::DBusBroker => {
                let path = tmp_dir.path().join("socket");
                let address = format!("unix:path={}", escape_path(&path));
                let socket = UnixListener::bind(&path)?;
                let process = Process::spawn_dbus_broker(
                    program,
                    &config_file,
                    socket.as_raw_fd(),
                )?;
                Ok(Daemon {
                    address,
                    tmp_dir,
                    process,
                })
            }
        }
    }
}

fn escape_path(path: &Path) -> String {
    use std::fmt::Write;

    let mut escaped = String::new();
    for b in path.as_os_str().as_bytes().iter().cloned() {
        match b {
            b'-'
            | b'0'..=b'9'
            | b'A'..=b'Z'
            | b'a'..=b'z'
            | b'_'
            | b'/'
            | b'.'
            | b'\\' => {
                escaped.push(b.into());
            }
            _ => {
                write!(&mut escaped, "%{0:2x}", b).unwrap();
            }
        }
    }

    escaped
}

impl Daemon {
    /// Returns the address of the message bus.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Returns the path to daemon configuration directory.
    ///
    /// The directory is temporary and removed after daemon is dropped.
    pub fn config_dir(&self) -> &Path {
        self.tmp_dir.path()
    }

    /// Returns the PID of the daemon process.
    pub fn pid(&self) -> libc::pid_t {
        self.process.pid()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.process.kill(libc::SIGTERM);
        let _ = self.process.try_wait_timeout(Duration::from_secs(10));
        let _ = self.process.kill(libc::SIGKILL);
        let _ = self.process.wait();
    }
}

impl Config {
    fn to_xml(&self) -> String {
        const DOCTYPE: &str = r#"<!DOCTYPE busconfig PUBLIC
 "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">"#;

        let mut s = String::new();
        s.push_str(DOCTYPE);
        s.push_str("\n");

        let mut xml = XmlWriter::new(&mut s);
        xml.start_tag("busconfig");

        if let Some(bus_type) = self.bus_type {
            xml.tag_with_text(
                "type",
                match bus_type {
                    BusType::Session => "session",
                    BusType::System => "system",
                },
            );
        }

        if self.allow_anonymous {
            xml.start_tag("allow_anonymous");
            xml.end_tag("allow_anonymous");
        }

        for listen in &self.listen {
            xml.tag_with_text("listen", listen);
        }

        for auth in &self.auth {
            xml.tag_with_text(
                "auth",
                match auth {
                    Auth::Anonymous => "ANONYMOUS",
                    Auth::External => "EXTERNAL",
                    Auth::DBusCookieSha1 => "DBUS_COOKIE_SHA1",
                },
            );
        }

        for dir in &self.service_dirs {
            let dir = dir.to_str().expect("servicedir is not valid UTF-8");
            xml.tag_with_text("servicedir", dir);
        }

        xml.start_tag("policy");
        xml.attr("context", "default");

        xml.start_tag("allow");
        xml.attr("receive_requested_reply", "true");
        xml.end_tag("allow");

        xml.start_tag("allow");
        xml.attr("send_destination", "*");
        xml.end_tag("allow");

        xml.start_tag("allow");
        xml.attr("own", "*");
        xml.end_tag("allow");

        xml.end_tag("policy");

        xml.end_tag("busconfig");

        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify xml config serialization.
    #[test]
    fn to_xml() {
        let mut c = Config::default();
        c.bus_type = Some(BusType::Session);
        c.listen.push("unix:tmpdir=/tmp".into());
        c.auth.push(Auth::Anonymous);
        c.auth.push(Auth::External);
        c.auth.push(Auth::DBusCookieSha1);
        c.service_dirs.push("/tmp/servicedir".into());

        let actual = c.to_xml();
        let expected = r#"<!DOCTYPE busconfig PUBLIC
 "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>ANONYMOUS</auth>
  <auth>EXTERNAL</auth>
  <auth>DBUS_COOKIE_SHA1</auth>
  <servicedir>/tmp/servicedir</servicedir>
  <policy context="default">
    <allow receive_requested_reply="true"/>
    <allow send_destination="*"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;

        assert_eq!(expected, actual, "\n\n{}.\n\n{}.", expected, actual);
    }

    #[test]
    fn escape() {
        assert_eq!("/", &escape_path(Path::new("/")));
        assert_eq!("/tmp/a%23b", &escape_path(Path::new("/tmp/a#b")));
    }
}
