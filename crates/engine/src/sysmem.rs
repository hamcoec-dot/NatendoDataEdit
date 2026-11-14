//! Installed memory, for limits that should scale with the computer.

/// Bytes of installed (physical) memory, if known.
#[cfg(windows)]
pub fn installed_bytes() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut st = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        dwMemoryLoad: 0,
        ullTotalPhys: 0,
        ullAvailPhys: 0,
        ullTotalPageFile: 0,
        ullAvailPageFile: 0,
        ullTotalVirtual: 0,
        ullAvailVirtual: 0,
        ullAvailExtendedVirtual: 0,
    };
    // SAFETY: `st` is a properly sized MEMORYSTATUSEX with dwLength set.
    #[allow(unsafe_code)]
    let ok = unsafe { GlobalMemoryStatusEx(&mut st) };
    (ok != 0 && st.ullTotalPhys > 0).then_some(st.ullTotalPhys)
}

/// Bytes of installed (physical) memory, if known.
#[cfg(not(windows))]
pub fn installed_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kb.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    #[test]
    fn installed_memory_is_plausible() {
        if let Some(b) = super::installed_bytes() {
            assert!(b >= 256 << 20, "{b}");
        }
        let cap = crate::pivot::pivot_budget();
        assert!(cap > 0 && cap <= crate::pivot::MAX_PIVOT_BYTES);
    }
}
