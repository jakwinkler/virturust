//! Linux namespace management for container isolation.
//!
//! Namespaces are the core kernel feature that provides isolation for containers.
//! Each namespace type isolates a different system resource:
//!
//! | Namespace | Flag            | What it isolates                         |
//! |-----------|-----------------|------------------------------------------|
//! | PID       | `CLONE_NEWPID`  | Process ID tree (container gets PID 1)   |
//! | Mount     | `CLONE_NEWNS`   | Mount points and filesystem view         |
//! | UTS       | `CLONE_NEWUTS`  | Hostname and domain name                 |
//! | IPC       | `CLONE_NEWIPC`  | Shared memory, semaphores, message queues|
//! | Network   | `CLONE_NEWNET`  | Network interfaces, routing, iptables    |
//!
//! Corten uses `clone()` with these flags to create a child process
//! that is born into all new namespaces simultaneously.

use anyhow::{anyhow, Context, Result};
use std::ffi::CString;

/// Arguments passed to the container's init process (PID 1).
///
/// These are heap-allocated and passed through `clone()` via the
/// arg pointer. Since `clone()` without `CLONE_VM` gives the child
/// its own address space (copy-on-write), both parent and child
/// can safely access their own copy.
pub struct ChildArgs {
    /// Absolute path to the prepared root filesystem
    pub rootfs: String,

    /// Hostname to set inside the container's UTS namespace
    pub hostname: String,

    /// Command and arguments to execute (argv)
    pub command: Vec<String>,

    /// Read end of the sync pipe — the child blocks on this until
    /// the parent has finished cgroup setup
    pub sync_pipe_rd: i32,

    /// Volume mounts to apply before pivot_root
    pub volumes: Vec<crate::config::VolumeMount>,

    /// Environment variables to set (KEY=VALUE format)
    pub env: Vec<String>,

    /// Working directory inside the container
    pub working_dir: String,

    /// User to run as (user or user:group)
    pub user: String,

    /// Network mode: "bridge", "none", or "host"
    pub network_mode: String,

    /// Path to redirect stdout to (for log capture)
    pub stdout_log: String,

    /// Path to redirect stderr to (for log capture)
    pub stderr_log: String,

    /// Whether to run in rootless mode (user namespace)
    pub rootless: bool,
}

/// Entry point for the cloned child process.
///
/// Called by `libc::clone()` on the child's new stack, inside all
/// new namespaces. This function bridges the C ABI to our Rust
/// container init logic.
///
/// # Safety
///
/// `arg` must point to a valid heap-allocated `ChildArgs`.
/// The child has its own copy of the address space (no `CLONE_VM`),
/// so accessing the data through this pointer is safe.
extern "C" fn child_entry(arg: *mut libc::c_void) -> libc::c_int {
    let args = unsafe { &*(arg as *const ChildArgs) };
    match child_main(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("corten: container init error: {e:#}");
            1
        }
    }
}

/// Main logic for the container init process (PID 1 inside the container).
///
/// Execution flow:
/// 1. Wait for parent to signal that cgroup setup is complete
/// 2. Set the container's hostname
/// 3. Set up the container's filesystem (mounts, pivot_root)
/// 4. Set up loopback networking
/// 5. exec() the requested command, replacing this process
fn child_main(args: &ChildArgs) -> Result<()> {
    // Block until parent signals that cgroup setup is done.
    // The parent writes a single byte to the pipe when ready.
    let mut buf = [0u8; 1];
    let ret = unsafe {
        libc::read(
            args.sync_pipe_rd,
            buf.as_mut_ptr() as *mut libc::c_void,
            1,
        )
    };
    if ret < 0 {
        return Err(anyhow!(
            "failed to read sync pipe: {}",
            std::io::Error::last_os_error()
        ));
    }
    unsafe { libc::close(args.sync_pipe_rd) };

    // Set container hostname (visible via `hostname` command inside container)
    let hostname = CString::new(args.hostname.as_str())
        .map_err(|_| anyhow!("invalid hostname"))?;
    if unsafe { libc::sethostname(hostname.as_ptr(), args.hostname.len()) } != 0 {
        return Err(anyhow!(
            "sethostname failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    // Open log file BEFORE pivot_root (host path won't be accessible after)
    // but don't redirect yet — we want setup logs to go to the real stderr
    let log_fd = if !args.stdout_log.is_empty() {
        use std::os::unix::io::IntoRawFd;
        std::fs::File::create(&args.stdout_log)
            .ok()
            .map(|f| f.into_raw_fd())
    } else {
        None
    };

    // Set up the container's filesystem (mount /proc, /sys, /dev, volumes, then pivot_root)
    crate::filesystem::setup_container_fs(&args.rootfs, &args.volumes)?;

    // Bring up loopback networking inside the network namespace
    crate::network::setup_loopback().ok(); // Non-fatal if `ip` command not available

    // Apply environment variables from image config
    for env_var in &args.env {
        if let Some((key, value)) = env_var.split_once('=') {
            // SAFETY: this runs in a single-threaded child process created by clone()
            // (no other threads exist), so mutating environment variables is safe.
            unsafe { std::env::set_var(key, value) };
        }
    }

    // Apply working directory
    if !args.working_dir.is_empty() {
        std::env::set_current_dir(&args.working_dir)
            .with_context(|| format!("failed to chdir to {}", args.working_dir))?;
    }

    // Security hardening: mask sensitive paths (after pivot_root)
    crate::security::mask_paths().ok();

    // Apply user (setgid then setuid — must set gid first)
    if !args.user.is_empty() {
        apply_user(&args.user)?;
    }

    // Security hardening: drop capabilities to safe default set (before exec)
    crate::security::drop_capabilities().ok();

    // Security hardening: apply seccomp-BPF filter (after caps, before exec)
    crate::security::apply_seccomp_filter().ok();

    // Build the argv for exec
    if args.command.is_empty() {
        return Err(anyhow!("no command specified"));
    }

    let cmd = CString::new(args.command[0].as_str())
        .map_err(|_| anyhow!("invalid command string: {:?}", args.command[0]))?;

    let c_args: Vec<CString> = args
        .command
        .iter()
        .map(|s| CString::new(s.as_str()).expect("invalid argument string"))
        .collect();

    let c_args_ptrs: Vec<*const libc::c_char> = c_args
        .iter()
        .map(|s| s.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    // Set up environment for exec (convert to C strings)
    let c_env: Vec<CString> = std::env::vars()
        .map(|(k, v)| CString::new(format!("{k}={v}")).expect("invalid env var"))
        .collect();

    let c_env_ptrs: Vec<*const libc::c_char> = c_env
        .iter()
        .map(|s| s.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    // Redirect stdout/stderr to log file right before exec
    // (all setup logging is done, only container output goes to the file)
    if let Some(fd) = log_fd {
        unsafe {
            libc::dup2(fd, 1); // stdout
            libc::dup2(fd, 2); // stderr
            libc::close(fd);
        }
    }

    // exec replaces this process entirely with the target command.
    // If exec returns, it failed.
    unsafe { libc::execvpe(cmd.as_ptr(), c_args_ptrs.as_ptr(), c_env_ptrs.as_ptr()) };

    Err(anyhow!(
        "execvpe failed for {:?}: {}",
        args.command[0],
        std::io::Error::last_os_error()
    ))
}

/// Apply user and group settings before exec.
///
/// Supports formats: "uid", "uid:gid", "user", "user:group".
/// Numeric values are used directly; names are looked up in /etc/passwd and /etc/group.
fn apply_user(user_spec: &str) -> Result<()> {
    let (user_part, group_part) = match user_spec.split_once(':') {
        Some((u, g)) => (u, Some(g)),
        None => (user_spec, None),
    };

    // Parse UID (numeric or lookup from /etc/passwd)
    let uid = if let Ok(uid) = user_part.parse::<u32>() {
        uid
    } else {
        lookup_uid(user_part)?
    };

    // Parse GID
    let gid = if let Some(group) = group_part {
        if let Ok(gid) = group.parse::<u32>() {
            Some(gid)
        } else {
            Some(lookup_gid(group)?)
        }
    } else {
        None
    };

    // Set GID first (must be done before dropping privileges via setuid)
    if let Some(gid) = gid {
        if unsafe { libc::setgid(gid) } != 0 {
            return Err(anyhow!("setgid({gid}) failed: {}", std::io::Error::last_os_error()));
        }
    }

    // Set UID
    if unsafe { libc::setuid(uid) } != 0 {
        return Err(anyhow!("setuid({uid}) failed: {}", std::io::Error::last_os_error()));
    }

    log::info!("switched to user {user_spec} (uid={uid}, gid={gid:?})");
    Ok(())
}

/// Look up a UID by username from /etc/passwd.
fn lookup_uid(username: &str) -> Result<u32> {
    let passwd = std::fs::read_to_string("/etc/passwd")
        .context("failed to read /etc/passwd")?;
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 3 && fields[0] == username {
            return fields[2]
                .parse()
                .map_err(|_| anyhow!("invalid UID in /etc/passwd for {username}"));
        }
    }
    Err(anyhow!("user '{username}' not found in /etc/passwd"))
}

/// Look up a GID by group name from /etc/group.
fn lookup_gid(groupname: &str) -> Result<u32> {
    let group = std::fs::read_to_string("/etc/group")
        .context("failed to read /etc/group")?;
    for line in group.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 3 && fields[0] == groupname {
            return fields[2]
                .parse()
                .map_err(|_| anyhow!("invalid GID in /etc/group for {groupname}"));
        }
    }
    Err(anyhow!("group '{groupname}' not found in /etc/group"))
}

/// Create a new process in fully isolated namespaces using `clone()`.
///
/// The child process is born into new PID, mount, UTS, IPC, and network
/// namespaces. It starts executing in [`child_entry`], which performs
/// container initialization and eventually exec's the target command.
///
/// The parent receives the child's PID (in the host PID namespace)
/// and is responsible for:
/// 1. Adding the child to the appropriate cgroup
/// 2. Signaling the child via the sync pipe
/// 3. Waiting for the child to exit
///
/// # Returns
///
/// The child's PID as seen from the host PID namespace.
pub fn create_namespaced_process(args: ChildArgs) -> Result<i32> {
    // Allocate a 1 MiB stack for the child process.
    // clone() requires its own stack since the child can't share
    // the parent's stack (they're separate processes).
    const STACK_SIZE: usize = 1024 * 1024;
    let mut stack = vec![0u8; STACK_SIZE];

    // x86_64 stacks grow downward, so we pass the TOP of the allocation
    let stack_top = unsafe { stack.as_mut_ptr().add(STACK_SIZE) };

    // Read rootless flag before boxing args
    let rootless = args.rootless;

    let mut flags = libc::CLONE_NEWPID
        | libc::CLONE_NEWNS
        | libc::CLONE_NEWUTS
        | libc::CLONE_NEWIPC
        | libc::SIGCHLD;

    // "host" mode shares the host network namespace
    if args.network_mode != "host" {
        flags |= libc::CLONE_NEWNET;
    }

    // Rootless mode: create a user namespace for unprivileged operation
    if rootless {
        flags |= libc::CLONE_NEWUSER;
    }

    // Heap-allocate the args so the pointer is valid in the child's
    // copy-on-write address space
    let args_box = Box::new(args);
    let args_ptr = Box::into_raw(args_box) as *mut libc::c_void;

    let pid = unsafe { libc::clone(child_entry, stack_top as *mut libc::c_void, flags, args_ptr) };

    if pid == -1 {
        // Reclaim the args to avoid a leak
        unsafe {
            let _ = Box::from_raw(args_ptr as *mut ChildArgs);
        }
        return Err(anyhow!(
            "clone() failed: {}. Are you running as root?",
            std::io::Error::last_os_error()
        ));
    }

    // The child has its own copy of the address space (COW), so we can
    // safely drop the parent's copy of the args and stack.
    unsafe {
        let _ = Box::from_raw(args_ptr as *mut ChildArgs);
    }
    // Leak the stack — the child process is actively using its COW copy.
    // The memory is reclaimed when the child exits.
    std::mem::forget(stack);

    log::info!("created namespaced child process with PID {pid}");
    Ok(pid)
}
