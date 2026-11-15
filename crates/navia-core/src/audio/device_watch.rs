//! Reports output device changes so a device list can refresh while it's open.
//! On Linux it listens to the pulse protocol, which PipeWire also serves. Elsewhere [`watch`] returns `None`.

/// Stops watching when dropped.
pub struct DeviceWatch {
    #[cfg(target_os = "linux")]
    socket: std::os::unix::net::UnixStream,
    #[cfg(target_os = "linux")]
    threads: Vec<std::thread::JoinHandle<()>>,
}

/// Changes that arrive this close together are reported once.
#[cfg(target_os = "linux")]
const SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

/// Calls `on_change` on a background thread whenever outputs or the default output change.
/// `None` when there is no sound server to watch.
#[cfg(target_os = "linux")]
pub fn watch(on_change: impl Fn() + Send + 'static) -> Option<DeviceWatch> {
    use std::io::BufReader;
    use std::os::unix::net::UnixStream;

    use pulseaudio::protocol::{self, Command, SubscriptionEventFacility as Facility, SubscriptionEventType as Kind, SubscriptionMask};

    let path = pulseaudio::socket_path_from_env()?;
    let stream = UnixStream::connect(path).ok()?;
    // The handshake must not hang the caller if the server is stuck.
    stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let cookie = pulseaudio::cookie_path_from_env().and_then(|p| std::fs::read(p).ok()).unwrap_or_default();
    let auth = protocol::AuthParams { version: protocol::MAX_VERSION, supports_shm: false, supports_memfd: false, cookie };
    protocol::write_command_message(reader.get_mut(), 0, &Command::Auth(auth), protocol::MAX_VERSION).ok()?;
    let (_, reply) = protocol::read_reply_message::<protocol::AuthReply>(&mut reader, protocol::MAX_VERSION).ok()?;
    let version = protocol::MAX_VERSION.min(reply.version);
    let mut props = protocol::Props::new();
    props.set(protocol::Prop::ApplicationName, c"Navia device watch");
    protocol::write_command_message(reader.get_mut(), 1, &Command::SetClientName(props), version).ok()?;
    protocol::read_reply_message::<protocol::SetClientNameReply>(&mut reader, version).ok()?;
    protocol::write_command_message(reader.get_mut(), 2, &Command::Subscribe(SubscriptionMask::SINK | SubscriptionMask::SERVER), version).ok()?;
    protocol::read_ack_message(&mut reader).ok()?;

    let relevant = |command: &Command| match command {
        Command::SubscribeEvent(e) => matches!((e.event_facility, e.event_type), (Facility::Sink, Kind::New | Kind::Removed) | (Facility::Server, _)),
        _ => false,
    };
    // The reader blocks on the socket, and the other thread lets a burst of changes settle.
    stream.set_read_timeout(None).ok()?;
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let reader_thread = std::thread::Builder::new()
        .name("navia-device-watch".into())
        .spawn(move || {
            while let Ok((_, command)) = protocol::read_command_message(&mut reader, version) {
                if relevant(&command) && tx.send(()).is_err() {
                    return;
                }
            }
        })
        .ok()?;
    let notify_thread = std::thread::Builder::new()
        .name("navia-device-notify".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                while rx.recv_timeout(SETTLE).is_ok() {}
                on_change();
            }
        })
        .ok()?;
    Some(DeviceWatch { socket: stream, threads: vec![reader_thread, notify_thread] })
}

#[cfg(not(target_os = "linux"))]
pub fn watch(_on_change: impl Fn() + Send + 'static) -> Option<DeviceWatch> {
    None
}

#[cfg(target_os = "linux")]
impl Drop for DeviceWatch {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
        self.threads.drain(..).for_each(|t| {
            let _ = t.join();
        });
    }
}
