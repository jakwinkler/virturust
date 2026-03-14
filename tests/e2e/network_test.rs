//! E2E tests for networking.

#[path = "../helpers/mod.rs"]
mod helpers;

#[test]
#[ignore = "requires root + cgroups v2 + network"]
fn container_has_outbound_connectivity() {
    if !helpers::require_root_and_cgroups() { return; }
    // TODO: run container, ping 8.8.8.8
}

#[test]
#[ignore = "requires root + cgroups v2 + network"]
fn port_forwarding_works() {
    if !helpers::require_root_and_cgroups() { return; }
    // TODO: run container with -p, curl from host
}

#[test]
#[ignore = "requires root + cgroups v2 + network"]
fn containers_communicate_on_same_bridge() {
    if !helpers::require_root_and_cgroups() { return; }
    // TODO: start two containers, ping between them
}

#[test]
#[ignore = "requires root + cgroups v2 + network"]
fn named_network_dns_resolves_container_names() {
    if !helpers::require_root_and_cgroups() { return; }
    // TODO: create network, start named containers, resolve by name
}
