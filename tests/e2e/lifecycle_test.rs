//! E2E tests for full container lifecycle.

#[path = "../helpers/mod.rs"]
mod helpers;

#[test]
#[ignore = "requires root + cgroups v2 + network"]
fn full_lifecycle_run_stop_inspect_rm() {
    if !helpers::require_root_and_cgroups() { return; }
    // TODO: run → stop → inspect → rm, verify each step
}

#[test]
#[ignore = "requires root + cgroups v2 + network"]
fn pull_and_run_real_image() {
    if !helpers::require_root_and_cgroups() { return; }
    // TODO: pull alpine, run cat /etc/os-release, verify output
}
