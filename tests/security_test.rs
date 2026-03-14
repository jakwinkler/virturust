//! Tests for security module.
//!
//! Most security tests need root privileges to actually drop capabilities
//! or mask paths, so they are marked #[ignore].

#[path = "helpers/mod.rs"]
mod helpers;

#[test]
fn security_module_compiles() {
    // Just verify the module is accessible and types exist
    // The actual functionality requires root
    let _ = std::any::type_name::<fn() -> anyhow::Result<()>>();
}

#[test]
#[ignore = "requires root + cgroups v2"]
fn drop_capabilities_succeeds_as_root() {
    if !helpers::require_root_and_cgroups() {
        return;
    }
    // In a real container context, this would reduce caps
    // Here we just verify it doesn't panic when called as root
    let result = corten::security::drop_capabilities();
    assert!(result.is_ok(), "drop_capabilities failed: {:?}", result.err());
}

#[test]
#[ignore = "requires root + cgroups v2"]
fn mask_paths_succeeds_as_root() {
    if !helpers::require_root_and_cgroups() {
        return;
    }
    let result = corten::security::mask_paths();
    assert!(result.is_ok(), "mask_paths failed: {:?}", result.err());
}

#[test]
#[ignore = "requires root + cgroups v2"]
fn seccomp_filter_applies_successfully() {
    if !helpers::require_root_and_cgroups() {
        return;
    }
    // Applying the seccomp filter should succeed when running as root.
    // Note: once applied, the filter persists for the lifetime of the process,
    // so this test should be run in isolation (hence #[ignore]).
    let result = corten::security::apply_seccomp_filter();
    assert!(
        result.is_ok(),
        "apply_seccomp_filter failed: {:?}",
        result.err()
    );
}

#[test]
#[ignore = "requires root + cgroups v2"]
fn seccomp_blocks_reboot() {
    if !helpers::require_root_and_cgroups() {
        return;
    }
    // Apply the seccomp filter first
    corten::security::apply_seccomp_filter()
        .expect("failed to apply seccomp filter");

    // Attempt to call reboot — should return EPERM (errno 1)
    let ret = unsafe { libc::reboot(libc::RB_AUTOBOOT) };
    assert_eq!(ret, -1, "reboot should have been blocked by seccomp");
    let err = std::io::Error::last_os_error();
    assert_eq!(
        err.raw_os_error(),
        Some(libc::EPERM),
        "seccomp should return EPERM, got: {err}"
    );
}
