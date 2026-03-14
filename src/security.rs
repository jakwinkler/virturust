//! Security hardening for containers.
//!
//! Implements capability dropping and path masking to reduce
//! the attack surface of containerized processes.

use anyhow::{anyhow, Result};

/// Linux capability constants (from <linux/capability.h>).
/// These are bit positions in the capability bitmask.
#[allow(dead_code)]
mod caps {
    pub const CAP_CHOWN: u32 = 0;
    pub const CAP_DAC_OVERRIDE: u32 = 1;
    pub const CAP_DAC_READ_SEARCH: u32 = 2;
    pub const CAP_FOWNER: u32 = 3;
    pub const CAP_FSETID: u32 = 4;
    pub const CAP_KILL: u32 = 5;
    pub const CAP_SETGID: u32 = 6;
    pub const CAP_SETUID: u32 = 7;
    pub const CAP_SETPCAP: u32 = 8;
    pub const CAP_LINUX_IMMUTABLE: u32 = 9;
    pub const CAP_NET_BIND_SERVICE: u32 = 10;
    pub const CAP_NET_BROADCAST: u32 = 11;
    pub const CAP_NET_ADMIN: u32 = 12;
    pub const CAP_NET_RAW: u32 = 13;
    pub const CAP_IPC_LOCK: u32 = 14;
    pub const CAP_IPC_OWNER: u32 = 15;
    pub const CAP_SYS_MODULE: u32 = 16;
    pub const CAP_SYS_RAWIO: u32 = 17;
    pub const CAP_SYS_CHROOT: u32 = 18;
    pub const CAP_SYS_PTRACE: u32 = 19;
    pub const CAP_SYS_PACCT: u32 = 20;
    pub const CAP_SYS_ADMIN: u32 = 21;
    pub const CAP_SYS_BOOT: u32 = 22;
    pub const CAP_SYS_NICE: u32 = 23;
    pub const CAP_SYS_RESOURCE: u32 = 24;
    pub const CAP_SYS_TIME: u32 = 25;
    pub const CAP_SYS_TTY_CONFIG: u32 = 26;
    pub const CAP_MKNOD: u32 = 27;
    pub const CAP_LEASE: u32 = 28;
    pub const CAP_AUDIT_WRITE: u32 = 29;
    pub const CAP_AUDIT_CONTROL: u32 = 30;
    pub const CAP_SETFCAP: u32 = 31;
    pub const CAP_LAST_CAP: u32 = 40;
}

/// The default set of capabilities to keep in a container.
/// This matches Docker's default capability set.
const DEFAULT_CAPS: &[u32] = &[
    caps::CAP_CHOWN,
    caps::CAP_DAC_OVERRIDE,
    caps::CAP_FOWNER,
    caps::CAP_FSETID,
    caps::CAP_KILL,
    caps::CAP_SETGID,
    caps::CAP_SETUID,
    caps::CAP_SETPCAP,
    caps::CAP_NET_BIND_SERVICE,
    caps::CAP_SYS_CHROOT,
    caps::CAP_MKNOD,
    caps::CAP_AUDIT_WRITE,
    caps::CAP_SETFCAP,
];

// PR_CAP_AMBIENT and related constants
const PR_CAP_AMBIENT: libc::c_int = 47;
const PR_CAP_AMBIENT_RAISE: libc::c_ulong = 2;
const PR_CAP_AMBIENT_CLEAR_ALL: libc::c_ulong = 4;

/// Drop all Linux capabilities except the default safe set.
///
/// This should be called in the child process right before exec().
/// It ensures the container process runs with minimal privileges.
///
/// The process:
/// 1. Clear the ambient capability set
/// 2. For each capability NOT in the default set, drop it from
///    the bounding, effective, permitted, and inheritable sets
/// 3. Re-raise the default caps in the ambient set so they survive exec
pub fn drop_capabilities() -> Result<()> {
    // Clear all ambient capabilities first
    if unsafe { libc::prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0) } != 0 {
        log::warn!(
            "failed to clear ambient capabilities: {}",
            std::io::Error::last_os_error()
        );
    }

    // Drop capabilities not in the default set from the bounding set
    for cap in 0..=caps::CAP_LAST_CAP {
        if DEFAULT_CAPS.contains(&cap) {
            continue;
        }
        // PR_CAPBSET_DROP = 24
        unsafe { libc::prctl(24, cap as libc::c_ulong, 0, 0, 0) };
    }

    // Build the capability bitmask for the default set
    let mut cap_mask: u64 = 0;
    for &cap in DEFAULT_CAPS {
        cap_mask |= 1u64 << cap;
    }

    // Set effective, permitted, and inheritable caps using capset
    // We use the v3 (64-bit) capability structure
    #[repr(C)]
    struct CapHeader {
        version: u32,
        pid: i32,
    }

    #[repr(C)]
    struct CapData {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }

    let header = CapHeader {
        version: 0x20080522, // _LINUX_CAPABILITY_VERSION_3
        pid: 0,              // current process
    };

    let data = [
        CapData {
            effective: cap_mask as u32,
            permitted: cap_mask as u32,
            inheritable: cap_mask as u32,
        },
        CapData {
            effective: (cap_mask >> 32) as u32,
            permitted: (cap_mask >> 32) as u32,
            inheritable: (cap_mask >> 32) as u32,
        },
    ];

    if unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) } != 0 {
        return Err(anyhow!(
            "capset failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    // Raise default caps in the ambient set so they survive exec
    for &cap in DEFAULT_CAPS {
        unsafe {
            libc::prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, cap as libc::c_ulong, 0, 0);
        }
    }

    log::info!("dropped capabilities to default set ({} caps retained)", DEFAULT_CAPS.len());
    Ok(())
}

/// Mask sensitive paths inside the container.
///
/// Bind-mounts `/dev/null` over sensitive procfs/sysfs entries to prevent
/// information leakage. Also makes certain paths read-only.
///
/// Must be called AFTER pivot_root and proc/sys are mounted.
pub fn mask_paths() -> Result<()> {
    // Paths to mask (bind /dev/null over them)
    let mask_targets = [
        "/proc/kcore",
        "/proc/keys",
        "/proc/timer_list",
        "/proc/sched_debug",
        "/sys/firmware",
    ];

    let null = std::ffi::CString::new("/dev/null").unwrap();

    for path in &mask_targets {
        let target = std::ffi::CString::new(*path).unwrap();
        if std::path::Path::new(path).exists() {
            let ret = unsafe {
                libc::mount(
                    null.as_ptr(),
                    target.as_ptr(),
                    std::ptr::null(),
                    libc::MS_BIND,
                    std::ptr::null(),
                )
            };
            if ret == 0 {
                log::debug!("masked {path}");
            }
        }
    }

    // Paths to make read-only
    let readonly_paths = ["/proc/sys", "/proc/irq", "/proc/bus"];

    for path in &readonly_paths {
        let target = std::ffi::CString::new(*path).unwrap();
        if std::path::Path::new(path).exists() {
            // Bind mount onto itself, then remount read-only
            unsafe {
                libc::mount(
                    target.as_ptr(),
                    target.as_ptr(),
                    std::ptr::null(),
                    libc::MS_BIND | libc::MS_REC,
                    std::ptr::null(),
                );
                libc::mount(
                    std::ptr::null(),
                    target.as_ptr(),
                    std::ptr::null(),
                    libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY | libc::MS_REC,
                    std::ptr::null(),
                );
            }
            log::debug!("made {path} read-only");
        }
    }

    log::info!("masked sensitive paths and made proc subsystems read-only");
    Ok(())
}
