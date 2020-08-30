use dbus_launch::{DaemonType, Launcher};
use std::ffi::OsStr;
use std::process::{Command, Stdio};

/// Unix transport is used by default.
#[test]
fn listen_default() {
    let daemon = Launcher::daemon().launch().unwrap();
    assert!(daemon.address().starts_with("unix:"));
}

/// TCP transport is used when requested.
#[test]
fn listen_tcp() {
    let daemon = Launcher::daemon()
        .listen("tcp:host=localhost")
        .launch()
        .unwrap();
    assert!(daemon.address().starts_with("tcp:"));
}

/// There can be multiple addresses to listen on.
#[test]
fn listen_tcp_and_unix() {
    let daemon = Launcher::daemon()
        .listen("tcp:host=localhost")
        .listen("unix:tmpdir=/tmp/")
        .launch()
        .unwrap();
    assert!(daemon.address().contains("unix:"));
    assert!(daemon.address().contains("tcp:"));
}

/// Verify that custom installed services are considered activatable by dbus-daemon.
fn service_support(daemon_type: DaemonType) {
    let mut launch = Launcher::new(daemon_type);
    let services: &[&str] = &["com.test.A", "com.test.B", "com.test.C"];
    for service in services {
        launch.service(service, "/usr/bin/false");
    }
    let daemon = launch.launch().unwrap();

    // Obtain the list of activatable names.
    let address = format!("--bus={}", daemon.address());
    let activatable = check_output(
        &"dbus-send",
        &[
            &address,
            "--print-reply",
            "--dest=org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.ListActivatableNames",
        ],
    );

    for service in services {
        assert!(
            activatable.contains(service),
            "Service {} should be among activatable names: {}",
            service,
            activatable,
        );
    }
}

#[test]
fn service_support_dbus() {
    service_support(DaemonType::DBusDaemon);
}

#[test]
fn service_support_broker() {
    if let Ok(_) = Command::new("dbus-broker").arg("--version").output() {
        service_support(DaemonType::DBusBroker);
    } else {
        println!("test ignored: dbus-broker --version failed")
    }
}

fn check_output<I, S>(program: S, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to execute child");
    let output = child.wait_with_output().expect("failed to wait on child");
    assert!(output.status.success(), "child process failed");
    String::from_utf8(output.stdout).expect("child output is not valid utf-8")
}
