//! Is the system clock disciplined (NTP / chrony / GPS-PPS)?

/// Linux: the kernel's NTP state (`adjtimex`) is synchronised. Other systems
/// manage their own clock: reported as synchronised.
#[cfg(target_os = "linux")]
pub fn synced() -> bool {
    // SAFETY: adjtimex with modes = 0 only reads the kernel state into `tx`.
    let mut tx: libc::timex = unsafe { std::mem::zeroed() };
    let state = unsafe { libc::adjtimex(&mut tx) };
    state >= 0 && state != libc::TIME_ERROR && tx.status & libc::STA_UNSYNC == 0
}

#[cfg(not(target_os = "linux"))]
pub fn synced() -> bool {
    true
}
