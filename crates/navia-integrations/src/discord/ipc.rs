//! The framing on Discord's local socket: a little-endian `u32` opcode, a little-endian `u32` length, then JSON.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

pub const OP_HANDSHAKE: u32 = 0;
pub const OP_FRAME: u32 = 1;
pub const OP_CLOSE: u32 = 2;
pub const OP_PING: u32 = 3;
pub const OP_PONG: u32 = 4;

const MAX_FRAME: usize = 1 << 20;
const PIPE_COUNT: u32 = 10;

pub struct Frame {
    pub opcode: u32,
    pub body: Value,
}

pub struct Connection {
    stream: platform::Stream,
    pending: Vec<u8>,
}

/// Where the Discord client puts its socket, in the order Discord documents.
/// Flatpak and Snap builds sit in subfolders of the runtime directory.
#[cfg(unix)]
pub fn default_socket_dirs() -> Vec<PathBuf> {
    use std::env;

    let mut bases: Vec<PathBuf> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .collect();
    bases.push(PathBuf::from("/tmp"));

    let mut dirs = Vec::new();
    for base in bases {
        let snaps: Vec<PathBuf> = std::fs::read_dir(&base)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("snap.discord"))
            .map(|entry| entry.path())
            .collect();
        dirs.push(base.clone());
        dirs.push(base.join("app/com.discordapp.Discord"));
        dirs.push(base.join(".flatpak/dev.vencord.Vesktop/xdg-run"));
        dirs.extend(snaps);
    }
    dirs
}

#[cfg(windows)]
pub fn default_socket_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from(r"\\?\pipe")]
}

impl Connection {
    /// Connects to the first Discord socket found in `dirs`.
    pub fn open(dirs: &[PathBuf]) -> Option<Self> {
        dirs.iter().find_map(|dir| Self::open_in(dir))
    }

    fn open_in(dir: &Path) -> Option<Self> {
        (0..PIPE_COUNT).find_map(|index| {
            let stream = platform::Stream::connect(&dir.join(format!("discord-ipc-{index}"))).ok()?;
            Some(Self {
                stream,
                pending: Vec::new(),
            })
        })
    }

    pub fn send(&mut self, opcode: u32, body: &Value) -> io::Result<()> {
        self.stream.write_all(&encode(opcode, body))
    }

    /// Returns the next frame, or `None` if none arrives within `timeout`.
    pub fn receive(&mut self, timeout: Duration) -> io::Result<Option<Frame>> {
        let started = std::time::Instant::now();
        loop {
            if let Some(frame) = take_frame(&mut self.pending)? {
                return Ok(Some(frame));
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            let mut chunk = [0u8; 4096];
            match self.stream.read_timeout(&mut chunk, remaining)? {
                Some(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Some(count) => self.pending.extend_from_slice(&chunk[..count]),
                None => return Ok(None),
            }
        }
    }
}

fn encode(opcode: u32, body: &Value) -> Vec<u8> {
    let json = body.to_string();
    let mut bytes = Vec::with_capacity(8 + json.len());
    bytes.extend_from_slice(&opcode.to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(json.as_bytes());
    bytes
}

fn take_frame(pending: &mut Vec<u8>) -> io::Result<Option<Frame>> {
    if pending.len() < 8 {
        return Ok(None);
    }
    let opcode = u32::from_le_bytes(pending[0..4].try_into().expect("four bytes"));
    let length = u32::from_le_bytes(pending[4..8].try_into().expect("four bytes")) as usize;
    if length > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Discord frame too large"));
    }
    if pending.len() < 8 + length {
        return Ok(None);
    }
    let body = serde_json::from_slice(&pending[8..8 + length])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    pending.drain(..8 + length);
    Ok(Some(Frame { opcode, body }))
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::os::unix::net::UnixStream;

    const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
    /// `set_read_timeout` rejects zero.
    const MIN_READ_TIMEOUT: Duration = Duration::from_millis(1);

    pub struct Stream(UnixStream);

    impl Stream {
        pub fn connect(path: &Path) -> io::Result<Self> {
            let stream = UnixStream::connect(path)?;
            stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
            Ok(Self(stream))
        }

        pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.0.write_all(bytes)
        }

        /// `Ok(Some(0))` means the other side closed. `Ok(None)` means timeout.
        pub fn read_timeout(&mut self, buffer: &mut [u8], timeout: Duration) -> io::Result<Option<usize>> {
            self.0.set_read_timeout(Some(timeout.max(MIN_READ_TIMEOUT)))?;
            match self.0.read(buffer) {
                Ok(count) => Ok(Some(count)),
                Err(error)
                    if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) =>
                {
                    Ok(None)
                }
                Err(error) => Err(error),
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::os::windows::io::AsRawHandle;
    use std::time::Instant;
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;

    const POLL: Duration = Duration::from_millis(10);

    /// A synchronous pipe handle can't read and write at once, so reads wait
    /// for data with `PeekNamedPipe` instead of blocking.
    pub struct Stream(File);

    impl Stream {
        pub fn connect(path: &Path) -> io::Result<Self> {
            Ok(Self(OpenOptions::new().read(true).write(true).open(path)?))
        }

        pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.0.write_all(bytes)
        }

        /// `Ok(Some(0))` means the other side closed. `Ok(None)` means timeout.
        pub fn read_timeout(&mut self, buffer: &mut [u8], timeout: Duration) -> io::Result<Option<usize>> {
            let started = Instant::now();
            loop {
                let mut available = 0u32;
                let ok = unsafe {
                    PeekNamedPipe(
                        self.0.as_raw_handle(),
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        &mut available,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    return Err(io::Error::last_os_error());
                }
                if available > 0 {
                    let count = buffer.len().min(available as usize);
                    return self.0.read(&mut buffer[..count]).map(Some);
                }
                if started.elapsed() >= timeout {
                    return Ok(None);
                }
                std::thread::sleep(POLL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_are_little_endian() {
        let bytes = encode(OP_FRAME, &json!({"a": 1}));
        assert_eq!(&bytes[..8], &[1, 0, 0, 0, 7, 0, 0, 0]);
        assert_eq!(&bytes[8..], br#"{"a":1}"#);
    }

    #[test]
    fn partial_frames_wait_for_more_bytes() {
        let bytes = encode(OP_PING, &json!({"n": "x"}));
        let mut pending = bytes[..10].to_vec();
        assert!(take_frame(&mut pending).unwrap().is_none());
        pending.extend_from_slice(&bytes[10..]);
        let frame = take_frame(&mut pending).unwrap().unwrap();
        assert_eq!(frame.opcode, OP_PING);
        assert_eq!(frame.body["n"], "x");
        assert!(pending.is_empty());
    }

    #[test]
    fn oversized_frames_are_rejected() {
        let mut pending = Vec::new();
        pending.extend_from_slice(&1u32.to_le_bytes());
        pending.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(take_frame(&mut pending).is_err());
    }
}
