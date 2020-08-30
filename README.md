# dbus-launch

An utility for starting an new isolated instance of a dbus-daemon or a
dbus-broker, with option to configure and start services using D-Bus
activation.

# Examples

## Launching a dbus-daemon process

```rust
// Start the dbus-daemon.
let daemon = dbus_launch::Launcher::daemon()
    .launch()
    .expect("failed to launch dbus-daemon");

// Use dbus-daemon by connecting to `daemon.address()` ...

// Stop the dbus-daemon process by dropping it.
drop(daemon);
```

## Starting services using D-Bus activation

```rust
use std::path::Path;

let daemon = dbus_launch::Launcher::daemon()
    .service("com.example.Test", Path::new("/usr/lib/test-service"))
    .launch()
    .expect("failed to launch dbus-daemon");

// Use com.example.Test service by connecting to `daemon.address()` ...
```

## License

Licensed under [MIT License](LICENSE-MIT).
