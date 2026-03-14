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
//! VirtuRust uses `clone()` with these flags to create a child process
//! that is born into all new namespaces simultaneously.

use anyhow::{anyhow, Result};
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
            eprintln!("virturust: container init error: {e:#}");
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

    // Set up the container's filesystem (mount /proc, /sys, /dev, then pivot_root)
    crate::filesystem::setup_container_fs(&args.rootfs)?;

    // Bring up loopback networking inside the network namespace
    crate::network::setup_loopback().ok(); // Non-fatal if `ip` command not available

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

    // exec replaces this process entirely with the target command.
    // If exec returns, it failed.
    unsafe { libc::execvp(cmd.as_ptr(), c_args_ptrs.as_ptr()) };

    Err(anyhow!(
        "execvp failed for {:?}: {}",
        args.command[0],
        std::io::Error::last_os_error()
    ))
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

    let flags = libc::CLONE_NEWPID
        | libc::CLONE_NEWNS
        | libc::CLONE_NEWUTS
        | libc::CLONE_NEWIPC
        | libc::CLONE_NEWNET
        | libc::SIGCHLD;

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
