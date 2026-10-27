// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
use crate::work_history::WorkSourceClock;
use std::sync::OnceLock;

pub(super) fn observe() -> Option<WorkSourceClock> {
    static BOOT: OnceLock<Option<String>> = OnceLock::new();
    let boot = BOOT.get_or_init(boot_identity).as_ref()?;
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // CLOCK_MONOTONIC is an OS-owned source shared by processes on this boot.
    // It is not process CPU time or a transferable Rust Instant.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut value) } != 0
        || value.tv_sec < 0
        || value.tv_nsec < 0
    {
        return None;
    }
    let nanoseconds = (value.tv_sec as u128)
        .checked_mul(1_000_000_000)?
        .checked_add(value.tv_nsec as u128)?;
    Some(WorkSourceClock {
        clock_id: format!("os_monotonic:{boot}"),
        nanoseconds: nanoseconds.to_string(),
    })
}

#[cfg(target_os = "macos")]
fn boot_identity() -> Option<String> {
    let mut bytes = [0u8; 128];
    let mut length = bytes.len();
    let result = unsafe {
        libc::sysctlbyname(
            c"kern.bootsessionuuid".as_ptr(),
            bytes.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 || length > bytes.len() {
        return None;
    }
    let value = std::str::from_utf8(&bytes[..length])
        .ok()?
        .trim_matches('\0');
    uuid::Uuid::parse_str(value).ok().map(|id| id.to_string())
}

#[cfg(target_os = "linux")]
fn boot_identity() -> Option<String> {
    use std::io::Read;
    let mut value = String::new();
    std::fs::File::open("/proc/sys/kernel/random/boot_id")
        .ok()?
        .take(128)
        .read_to_string(&mut value)
        .ok()?;
    uuid::Uuid::parse_str(value.trim())
        .ok()
        .map(|id| id.to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn boot_identity() -> Option<String> {
    None
}
