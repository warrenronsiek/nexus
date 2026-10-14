// @feature configuration
// @feature coordination
// @feature analyst
// @feature commit-review
// @feature agent-memory
// @spec docs/features/configuration.md
// @spec docs/features/coordination.md
// @spec docs/features/analyst.md
// @spec docs/features/commit-review.md
// @spec docs/features/agent-memory.md
// @boundary bounded-child-process-output
use std::io::Read;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const CAPTURE_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) type OutputReader = std::thread::JoinHandle<std::io::Result<CapturedOutput>>;

pub(crate) struct CapturedOutput {
    pub(crate) bytes: Vec<u8>,
    pub(crate) exceeded: bool,
}

pub(crate) fn spawn_bounded_capture<R, F>(
    reader: R,
    byte_limit: usize,
    cancellation: Arc<AtomicBool>,
    on_limit: F,
) -> OutputReader
where
    R: Read + Send + 'static,
    F: FnOnce() + Send + 'static,
{
    std::thread::spawn(move || {
        let output = capture_output(reader, byte_limit, &cancellation)?;
        if output.exceeded {
            on_limit();
        }
        Ok(output)
    })
}

fn capture_output(
    mut reader: impl Read,
    byte_limit: usize,
    cancellation: &AtomicBool,
) -> std::io::Result<CapturedOutput> {
    let mut bytes = Vec::with_capacity(byte_limit.min(8_192));
    let mut buffer = [0_u8; 8_192];
    loop {
        let remaining = byte_limit.saturating_sub(bytes.len());
        let read_limit = remaining.saturating_add(1).min(buffer.len());
        let count = match reader.read(&mut buffer[..read_limit]) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if cancellation.load(Ordering::Acquire) {
                    return Ok(CapturedOutput {
                        bytes,
                        exceeded: false,
                    });
                }
                std::thread::sleep(CAPTURE_POLL_INTERVAL);
                continue;
            }
            Err(error) => return Err(error),
        };
        if count == 0 {
            return Ok(CapturedOutput {
                bytes,
                exceeded: false,
            });
        }
        if count > remaining {
            bytes.extend_from_slice(&buffer[..remaining]);
            return Ok(CapturedOutput {
                bytes,
                exceeded: true,
            });
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

pub(crate) fn set_nonblocking(descriptor: &impl AsRawFd) -> std::io::Result<()> {
    let file_descriptor = descriptor.as_raw_fd();
    // SAFETY: fcntl only reads or updates flags for the valid descriptor
    // borrowed from the caller for the duration of this function.
    let flags = unsafe { libc::fcntl(file_descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the borrowed descriptor remains open for this call and the
    // existing flags are preserved while adding O_NONBLOCK.
    if unsafe { libc::fcntl(file_descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{set_nonblocking, spawn_bounded_capture};
    use std::io::{Cursor, Read};
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn capture_is_exactly_bounded_and_notifies_once() {
        let notifications = Arc::new(AtomicUsize::new(0));
        let observed_notifications = notifications.clone();
        let reader = spawn_bounded_capture(
            Cursor::new(b"abcdef"),
            3,
            Arc::new(AtomicBool::new(false)),
            move || {
                observed_notifications.fetch_add(1, Ordering::Relaxed);
            },
        );

        let output = reader.join().unwrap().unwrap();

        assert_eq!(output.bytes, b"abc");
        assert!(output.exceeded);
        assert_eq!(notifications.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn configured_descriptor_reports_would_block_without_input() {
        let (mut reader, _writer) = UnixStream::pair().unwrap();
        set_nonblocking(&reader).unwrap();

        let error = reader.read(&mut [0_u8; 1]).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    }
}
