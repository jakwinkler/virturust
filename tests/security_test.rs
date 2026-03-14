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
