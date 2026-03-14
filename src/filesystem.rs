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
///   (e.g., `/var/lib/corten/images/alpine/latest/rootfs`)
pub fn setup_container_fs(rootfs: &str, volumes: &[crate::config::VolumeMount]) -> Result<()> {
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

    // Step 3b: Mount volumes (bind mounts from host into container rootfs).
    // This must happen BEFORE pivot_root because host paths are still accessible.
    mount_volumes(rootfs, volumes)?;

    // Step 4: pivot_root — make rootfs the new "/" and stash old "/" at /old_root.
    let old_root = rootfs.join("old_root");
    fs::create_dir_all(&old_root).context("failed to create old_root mount point")?;

    let new_root = path_to_cstring(rootfs).context("invalid rootfs path for pivot_root")?;
    let put_old = path_to_cstring(&old_root).context("invalid old_root path for pivot_root")?;

    if unsafe {
        libc::syscall(
            libc::SYS_pivot_root,
            new_root.as_ptr(),
            put_old.as_ptr(),
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

    // /dev — minimal device nodes (NOT bind-mounted from host for security)
    let dev_dir = rootfs.join("dev");
    fs::create_dir_all(&dev_dir).context("failed to create /dev")?;
    setup_minimal_dev(&dev_dir).context("failed to set up minimal /dev")?;

    Ok(())
}

/// Mount volume bind mounts into the container rootfs.
///
/// Each volume is bind-mounted from the host path to the container path
/// (relative to rootfs). For read-only volumes, a remount with MS_RDONLY
/// is applied after the initial bind mount.
fn mount_volumes(rootfs: &Path, volumes: &[crate::config::VolumeMount]) -> Result<()> {
    for vol in volumes {
        let target = rootfs.join(
            vol.container_path
                .strip_prefix("/")
                .unwrap_or(&vol.container_path),
        );

        fs::create_dir_all(&target)
            .with_context(|| format!("failed to create mount point {}", target.display()))?;

        bind_mount(&vol.host_path, &target)
            .with_context(|| format!(
                "failed to bind mount {} -> {}",
                vol.host_path.display(),
                vol.container_path.display()
            ))?;

        if vol.read_only {
            remount_readonly(&target)
                .with_context(|| format!(
                    "failed to remount {} as read-only",
                    vol.container_path.display()
                ))?;
        }

        log::info!(
            "mounted volume {} -> {}{}",
            vol.host_path.display(),
            vol.container_path.display(),
            if vol.read_only { " (ro)" } else { "" }
        );
    }
    Ok(())
}

/// Remount a bind mount as read-only.
fn remount_readonly(target: &Path) -> Result<()> {
    let tgt = path_to_cstring(target)?;

    if unsafe {
        libc::mount(
            std::ptr::null(),
            tgt.as_ptr(),
            std::ptr::null(),
            libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY,
            std::ptr::null(),
        )
    } != 0
    {
        return Err(anyhow!(
            "remount readonly on {} failed: {}",
            target.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// Set up a minimal /dev with only the devices containers need.
///
/// Creates a tmpfs at the /dev mount point and populates it with
/// essential device nodes, symlinks, and pseudo-filesystems.
/// This is much safer than bind-mounting the host /dev.
fn setup_minimal_dev(dev_dir: &Path) -> Result<()> {
    // Mount tmpfs on /dev
    mount_fs("tmpfs", dev_dir, "tmpfs", libc::MS_NOSUID | libc::MS_STRICTATIME)
        .context("failed to mount tmpfs on /dev")?;

    // Create essential device nodes via mknod
    // (major, minor) pairs follow Linux device number conventions
    let devices: &[(&str, u32, u32, u32)] = &[
        // (name, mode, major, minor)
        ("null",    0o666, 1, 3),
        ("zero",    0o666, 1, 5),
        ("random",  0o666, 1, 8),
        ("urandom", 0o666, 1, 9),
        ("tty",     0o666, 5, 0),
        ("console", 0o600, 5, 1),
    ];

    for (name, mode, major, minor) in devices {
        let path = dev_dir.join(name);
        let dev = libc::makedev(*major, *minor);
        let c_path = path_to_cstring(&path)?;
        // S_IFCHR = character device
        if unsafe { libc::mknod(c_path.as_ptr(), libc::S_IFCHR | mode, dev) } != 0 {
            let err = std::io::Error::last_os_error();
            // EPERM is expected in user namespaces — bind mount from host instead
            if err.raw_os_error() == Some(libc::EPERM) {
                let host_dev = Path::new("/dev").join(name);
                if host_dev.exists() {
                    bind_mount(&host_dev, &path)
                        .with_context(|| format!("failed to bind mount /dev/{name}"))?;
                    continue;
                }
            }
            log::warn!("mknod /dev/{name} failed: {err}");
        }
    }

    // /dev/pts — pseudo-terminal slave devices
    let pts_dir = dev_dir.join("pts");
    fs::create_dir_all(&pts_dir).context("failed to create /dev/pts")?;
    mount_fs(
        "devpts",
        &pts_dir,
        "devpts",
        libc::MS_NOSUID | libc::MS_NOEXEC,
    )
    .context("failed to mount devpts")
    .ok(); // Non-fatal: devpts may not be available

    // /dev/ptmx → pts/ptmx symlink
    std::os::unix::fs::symlink("pts/ptmx", dev_dir.join("ptmx")).ok();

    // /dev/shm — shared memory
    let shm_dir = dev_dir.join("shm");
    fs::create_dir_all(&shm_dir).context("failed to create /dev/shm")?;
    mount_fs("tmpfs", &shm_dir, "tmpfs", libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC)
        .context("failed to mount /dev/shm")
        .ok(); // Non-fatal

    // Standard I/O symlinks → /proc/self/fd/*
    std::os::unix::fs::symlink("/proc/self/fd", dev_dir.join("fd")).ok();
    std::os::unix::fs::symlink("/proc/self/fd/0", dev_dir.join("stdin")).ok();
    std::os::unix::fs::symlink("/proc/self/fd/1", dev_dir.join("stdout")).ok();
    std::os::unix::fs::symlink("/proc/self/fd/2", dev_dir.join("stderr")).ok();

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

/// Set up an OverlayFS mount for a container.
///
/// Creates `upper/`, `work/`, and `merged/` directories under `overlay_dir`,
/// then mounts OverlayFS with the image rootfs as the read-only lower layer.
///
/// The `merged/` directory becomes the container's rootfs.
pub fn setup_overlay(image_rootfs: &Path, overlay_dir: &Path) -> Result<()> {
    let upper = overlay_dir.join("upper");
    let work = overlay_dir.join("work");
    let merged = overlay_dir.join("merged");

    fs::create_dir_all(&upper).context("failed to create overlay upper dir")?;
    fs::create_dir_all(&work).context("failed to create overlay work dir")?;
    fs::create_dir_all(&merged).context("failed to create overlay merged dir")?;

    let options = format!(
        "lowerdir={},upperdir={},workdir={}",
        image_rootfs.display(),
        upper.display(),
        work.display()
    );

    let src = std::ffi::CString::new("overlay").unwrap();
    let tgt = path_to_cstring(&merged)?;
    let fst = std::ffi::CString::new("overlay").unwrap();
    let opts = std::ffi::CString::new(options.as_str())
        .map_err(|_| anyhow!("overlay options contain null byte"))?;

    if unsafe {
        libc::mount(
            src.as_ptr(),
            tgt.as_ptr(),
            fst.as_ptr(),
            0,
            opts.as_ptr() as *const libc::c_void,
        )
    } != 0
    {
        return Err(anyhow!(
            "OverlayFS mount failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    log::info!("mounted OverlayFS at {}", merged.display());
    Ok(())
}

/// Clean up an OverlayFS mount.
pub fn cleanup_overlay(overlay_dir: &Path) -> Result<()> {
    let merged = overlay_dir.join("merged");
    if merged.exists() {
        let tgt = path_to_cstring(&merged)?;
        unsafe { libc::umount2(tgt.as_ptr(), libc::MNT_DETACH) };
        log::info!("unmounted OverlayFS at {}", merged.display());
    }
    Ok(())
}

/// Convert a Path to a null-terminated CString for use with libc functions.
fn path_to_cstring(path: &Path) -> Result<std::ffi::CString> {
    let bytes = path.as_os_str().as_encoded_bytes();
    std::ffi::CString::new(bytes.to_vec())
        .map_err(|_| anyhow!("path contains null byte: {}", path.display()))
}
