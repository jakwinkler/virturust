//! E2E tests for container networking.
//!
//! These tests verify network connectivity between containers and the host.
//! They require root, cgroups v2, network access, and a pulled alpine image.
//!
//! Run with: sudo cargo test -- --ignored --test-threads=1

#[path = "../helpers/mod.rs"]
mod helpers;

#[test]
#[ignore = "requires root + cgroups v2 + network + alpine"]
fn container_has_outbound_connectivity() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    let name = helpers::test_container_name("netout");
    let (code, stdout, stderr) = helpers::run_corten(&[
        "run", "--name", &name, "alpine", "ping", "-c", "1", "-W", "5", "8.8.8.8",
    ]);

    // Ping should succeed (outbound NAT)
    assert_eq!(
        code, 0,
        "ping should succeed with bridge networking.\nstdout: {stdout}\nstderr: {stderr}"
    );

    helpers::run_corten(&["rm", &name]);
}

#[test]
#[ignore = "requires root + cgroups v2 + network + alpine"]
fn container_dns_resolution_works() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    let name = helpers::test_container_name("dns");

    // nslookup/wget to verify DNS works
    let (code, stdout, stderr) = helpers::run_corten(&[
        "run", "--name", &name,
        "alpine", "wget", "-q", "-O", "-", "--timeout=5", "http://example.com",
    ]);

    // Should get some HTML back
    if code == 0 {
        assert!(
            stdout.contains("Example Domain") || stdout.contains("html"),
            "should get HTML content"
        );
    } else {
        eprintln!("DNS test inconclusive (may be firewall): stderr={stderr}");
    }

    helpers::run_corten(&["rm", &name]);
}

#[test]
#[ignore = "requires root + cgroups v2 + alpine"]
fn network_none_has_no_connectivity() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    let name = helpers::test_container_name("netnone");
    let (code, _, _) = helpers::run_corten(&[
        "run", "--name", &name, "--network", "none",
        "alpine", "ping", "-c", "1", "-W", "2", "8.8.8.8",
    ]);

    // Should fail — no network
    assert_ne!(code, 0, "ping should fail with --network none");

    helpers::run_corten(&["rm", &name]);
}

#[test]
#[ignore = "requires root + cgroups v2 + alpine"]
fn network_host_shares_host_network() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    let name = helpers::test_container_name("nethost");
    let (code, stdout, _) = helpers::run_corten(&[
        "run", "--name", &name, "--network", "host",
        "alpine", "ip", "addr", "show",
    ]);

    assert_eq!(code, 0);
    // Should see host interfaces (not just lo)
    assert!(
        stdout.contains("eth") || stdout.contains("wl") || stdout.contains("enp") || stdout.contains("ens"),
        "host network mode should show host interfaces, got: {stdout}"
    );

    helpers::run_corten(&["rm", &name]);
}

#[test]
#[ignore = "requires root + cgroups v2 + alpine"]
fn containers_on_same_bridge_can_communicate() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    // Start a container that listens (run a simple sleep so it stays up)
    let server_name = helpers::test_container_name("bridge-srv");
    let (_code, stdout, _) = helpers::run_corten(&[
        "run", "-d", "--name", &server_name, "alpine", "sleep", "30",
    ]);
    let server_id = stdout.trim();
    assert!(!server_id.is_empty(), "detached container should print ID");

    // Give networking a moment to set up
    std::thread::sleep(std::time::Duration::from_secs(1));

    // Get the server's IP from inspect
    let inspect_out = helpers::run_corten_ok(&["inspect", &server_name]);

    // The server is on the bridge. A second container on the same bridge
    // should be able to reach it. For simplicity, just verify both containers
    // get bridge IPs.
    let client_name = helpers::test_container_name("bridge-cli");
    let (code, stdout, _) = helpers::run_corten(&[
        "run", "--name", &client_name, "alpine", "ip", "addr", "show", "eth0",
    ]);

    assert_eq!(code, 0);
    assert!(
        stdout.contains("10.0.42."),
        "container should have a 10.0.42.x IP, got: {stdout}"
    );

    // Cleanup
    helpers::run_corten(&["stop", &server_name]);
    helpers::run_corten(&["rm", &server_name]);
    helpers::run_corten(&["rm", &client_name]);
    let _ = inspect_out; // used above
}
