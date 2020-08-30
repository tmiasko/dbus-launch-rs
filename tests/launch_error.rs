//! Verifies that exec error code is returned on launch failure.

fn main() {
    std::env::set_var("PATH", "/non-existing-directory/");

    let error = dbus_launch::Launcher::daemon().launch().unwrap_err();
    assert_eq!(std::io::ErrorKind::NotFound, error.kind());

    let error = dbus_launch::Launcher::broker().launch().unwrap_err();
    assert_eq!(std::io::ErrorKind::NotFound, error.kind());
}
