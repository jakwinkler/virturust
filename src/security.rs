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

// --- Seccomp-BPF syscall filtering ---

/// BPF filter instruction (matches `struct sock_filter` from <linux/filter.h>).
#[repr(C)]
struct SockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

/// BPF filter program (matches `struct sock_fprog` from <linux/filter.h>).
#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *const SockFilter,
}

// BPF instruction class/mode constants
const BPF_LD: u16 = 0x00;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JMP: u16 = 0x05;
const BPF_JEQ: u16 = 0x10;
const BPF_K_JMP: u16 = 0x00;
const BPF_RET: u16 = 0x06;
const BPF_K_RET: u16 = 0x00;

// Seccomp return action constants
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;

/// x86_64 syscall numbers for dangerous calls that containers should never need.
const BLOCKED_SYSCALLS: &[u32] = &[
    246, // kexec_load
    320, // kexec_file_load
    169, // reboot
    167, // swapon
    168, // swapoff
    175, // init_module
    313, // finit_module
    176, // delete_module
    163, // acct
    227, // clock_settime
    305, // clock_adjtime
    164, // settimeofday
    159, // adjtimex
    170, // sethostname
    171, // setdomainname
];

/// Apply a seccomp-BPF filter to restrict dangerous syscalls.
///
/// Blocks syscalls that containers should never need:
/// - `kexec_load`, `kexec_file_load`: load a new kernel
/// - `reboot`: reboot the system
/// - `swapon`, `swapoff`: manage swap space
/// - `init_module`, `finit_module`, `delete_module`: kernel module operations
/// - `acct`: process accounting
/// - `clock_settime`, `clock_adjtime`, `settimeofday`, `adjtimex`: time manipulation
/// - `sethostname`, `setdomainname`: hostname changes (already set during init)
///
/// Uses `SECCOMP_RET_ERRNO` (returns `EPERM`) rather than `SECCOMP_RET_KILL`
/// for better debuggability.
///
/// Must be called after `drop_capabilities()` and before `exec()`.
pub fn apply_seccomp_filter() -> Result<()> {
    // Build the BPF filter program.
    //
    // Structure:
    //   [0]   Load syscall number from seccomp_data.nr (offset 0)
    //   [1..N] For each blocked syscall: JEQ <nr> -> jump to DENY, else fall through
    //   [N+1] ALLOW (default action)
    //   [N+2] DENY  (return EPERM)
    let num_blocked = BLOCKED_SYSCALLS.len();
    let mut filter: Vec<SockFilter> = Vec::with_capacity(num_blocked * 2 + 2);

    // Instruction 0: load the syscall number (seccomp_data.nr is at offset 0)
    filter.push(SockFilter {
        code: BPF_LD | BPF_W | BPF_ABS,
        jt: 0,
        jf: 0,
        k: 0, // offsetof(seccomp_data, nr)
    });

    // For each blocked syscall, add a JEQ instruction.
    // If it matches, jump forward to the DENY instruction at the end.
    // If it doesn't match, fall through to the next check (jf=0).
    //
    // After instruction 0 (the load), instructions [1..num_blocked] are the JEQ checks.
    // Instruction [num_blocked + 1] is ALLOW.
    // Instruction [num_blocked + 2] is DENY.
    //
    // From JEQ instruction at index `i` (1-based), the DENY instruction is at
    // index (num_blocked + 2). The jump offset from instruction i is:
    //   (num_blocked + 2) - (i + 1) = num_blocked + 1 - i
    // But `i` ranges from 1 to num_blocked, so the jt offset is:
    //   num_blocked + 1 - i  (for 1-indexed i)
    // Or equivalently, for the k-th blocked syscall (0-indexed):
    //   jt = (num_blocked - 1 - k) + 1 = num_blocked - k
    // The jf is always 0 (fall through to next instruction).
    for (k, &syscall_nr) in BLOCKED_SYSCALLS.iter().enumerate() {
        let jump_to_deny = (num_blocked - k) as u8;
        filter.push(SockFilter {
            code: BPF_JMP | BPF_JEQ | BPF_K_JMP,
            jt: jump_to_deny,
            jf: 0,
            k: syscall_nr,
        });
    }

    // ALLOW: default action for non-blocked syscalls
    filter.push(SockFilter {
        code: BPF_RET | BPF_K_RET,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ALLOW,
    });

    // DENY: return EPERM (errno 1) for blocked syscalls
    filter.push(SockFilter {
        code: BPF_RET | BPF_K_RET,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ERRNO | 1, // EPERM = 1
    });

    // Set PR_SET_NO_NEW_PRIVS — required before installing a seccomp filter
    // as a non-privileged process (and good practice regardless).
    const PR_SET_NO_NEW_PRIVS: libc::c_int = 38;
    if unsafe { libc::prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(anyhow!(
            "prctl(PR_SET_NO_NEW_PRIVS) failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    // Install the seccomp filter via the seccomp() syscall.
    // SECCOMP_SET_MODE_FILTER = 1, flags = 0
    let prog = SockFprog {
        len: filter.len() as u16,
        filter: filter.as_ptr(),
    };
    if unsafe { libc::syscall(libc::SYS_seccomp, 1u64, 0u64, &prog as *const _) } != 0 {
        return Err(anyhow!(
            "seccomp(SECCOMP_SET_MODE_FILTER) failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    log::info!(
        "applied seccomp-BPF filter ({} syscalls blocked)",
        BLOCKED_SYSCALLS.len()
    );
    Ok(())
}
