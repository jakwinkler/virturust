//! Container filesystem setup using mount namespaces and pivot_root.
//!
//! This module handles the critical filesystem isolation that makes a
//! container feel like its own machine. The process:
//!
//! 1. **Private mounts** — prevent mount events from propagating to the host
//! 2. **Bind mount rootfs** — `pivot_root` requires the new root to be a mount point
//! 3. **Mount virtual filesystems** — `/proc`, `/sys`, `/dev` inside the container
//! 4. **pivot_root** — swap the root filesystem so the container can't see the host
//! 5. **Cleanup** — unmount and remove the old root

use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::Path;

/// Set up the container's filesystem isolation.
///
/// Called from inside the container's mount namespace (after `clone(CLONE_NEWNS)`).
/// After this function completes, the process's root filesystem is the
/// container's rootfs, and the host filesystem is completely inaccessible.
///
/// # Arguments
///
/// * `rootfs` — Absolute path to the extracted image root filesystem
///   (e.g., `/var/lib/virturust/images/alpine/latest/rootfs`)
pub fn setup_container_fs(rootfs: &str) -> Result<()> {
    let rootfs = Path::new(rootfs);

    if !rootfs.exists() {
        return Err(anyhow!("rootfs does not exist: {}", rootfs.display()));
    }

    // Step 1: Make the entire mount tree private.
    // This prevents any mount/unmount events inside the container from
    // propagating to the host, and vice versa.
    mount_none("/", libc::MS_REC | libc::MS_PRIVATE)
        .context("failed to make mounts private")?;

    // Step 2: Bind-mount rootfs onto itself.
    // pivot_root(2) requires that new_root is a mount point. A bind mount
    // of a directory onto itself satisfies this requirement.
    bind_mount(rootfs, rootfs).context("failed to bind mount rootfs")?;

    // Step 3: Create mount points and mount virtual filesystems.
    mount_special_filesystems(rootfs)?;

    // Step 4: pivot_root — make rootfs the new "/" and stash old "/" at /old_root.
    let old_root = rootfs.join("old_root");
    fs::create_dir_all(&old_root).context("failed to create old_root mount point")?;

    if unsafe {
        libc::syscall(
            libc::SYS_pivot_root,
            rootfs.as_os_str().as_encoded_bytes().as_ptr(),
            old_root.as_os_str().as_encoded_bytes().as_ptr(),
        )
    } == -1
    {
        return Err(anyhow!(
            "pivot_root failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    // Step 5: chdir to new root
    std::env::set_current_dir("/").context("failed to chdir to /")?;

    // Step 6: Unmount the old root (lazy/detach — handles busy mounts)
    if unsafe { libc::umount2(b"/old_root\0".as_ptr() as *const _, libc::MNT_DETACH) } != 0 {
        log::warn!(
            "failed to unmount /old_root: {}",
            std::io::Error::last_os_error()
        );
    }

    // Step 7: Remove the empty old_root directory
    fs::remove_dir("/old_root").ok(); // Best-effort, may fail if not empty

    Ok(())
}

/// Mount /proc, /sys, and /dev inside the new rootfs.
fn mount_special_filesystems(rootfs: &Path) -> Result<()> {
    // /proc — process information pseudo-filesystem
    let proc_dir = rootfs.join("proc");
    fs::create_dir_all(&proc_dir).context("failed to create /proc")?;
    mount_fs(
        "proc",
        &proc_dir,
        "proc",
        libc::MS_NOSUID | libc::MS_NOEXEC | libc::MS_NODEV,
    )
    .context("failed to mount /proc")?;

    // /sys — kernel and device information (read-only for safety)
    let sys_dir = rootfs.join("sys");
    fs::create_dir_all(&sys_dir).context("failed to create /sys")?;
    mount_fs(
        "sysfs",
        &sys_dir,
        "sysfs",
        libc::MS_NOSUID | libc::MS_NOEXEC | libc::MS_NODEV | libc::MS_RDONLY,
    )
    .context("failed to mount /sys")?;

    // /dev — device nodes (bind mount from host)
    let dev_dir = rootfs.join("dev");
    fs::create_dir_all(&dev_dir).context("failed to create /dev")?;
    bind_mount(Path::new("/dev"), &dev_dir).context("failed to bind mount /dev")?;

    Ok(())
}

/// Perform a bind mount (mount --bind).
fn bind_mount(source: &Path, target: &Path) -> Result<()> {
    let src = path_to_cstring(source)?;
    let tgt = path_to_cstring(target)?;

    if unsafe {
        libc::mount(
            src.as_ptr(),
            tgt.as_ptr(),
            std::ptr::null(),
            libc::MS_BIND | libc::MS_REC,
            std::ptr::null(),
        )
    } != 0
    {
        return Err(anyhow!(
            "bind mount {} -> {} failed: {}",
            source.display(),
            target.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// Mount a filesystem by type.
fn mount_fs(source: &str, target: &Path, fstype: &str, flags: u64) -> Result<()> {
    let src = std::ffi::CString::new(source).unwrap();
    let tgt = path_to_cstring(target)?;
    let fst = std::ffi::CString::new(fstype).unwrap();

    if unsafe {
        libc::mount(
            src.as_ptr(),
            tgt.as_ptr(),
            fst.as_ptr(),
            flags,
            std::ptr::null(),
        )
    } != 0
    {
        return Err(anyhow!(
            "mount {source} on {} ({fstype}) failed: {}",
            target.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// Mount with no source/fstype, just flags (e.g., making mounts private).
fn mount_none(target: &str, flags: u64) -> Result<()> {
    let tgt = std::ffi::CString::new(target).unwrap();

    if unsafe {
        libc::mount(
            std::ptr::null(),
            tgt.as_ptr(),
            std::ptr::null(),
            flags,
            std::ptr::null(),
        )
    } != 0
    {
        return Err(anyhow!(
            "mount(flags={flags:#x}) on {target} failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// Convert a Path to a null-terminated CString for use with libc functions.
fn path_to_cstring(path: &Path) -> Result<std::ffi::CString> {
    let bytes = path.as_os_str().as_encoded_bytes();
    std::ffi::CString::new(bytes.to_vec())
        .map_err(|_| anyhow!("path contains null byte: {}", path.display()))
}
