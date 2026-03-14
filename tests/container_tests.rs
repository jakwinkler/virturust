//! Tests for container lifecycle and state management.
//!
//! Unit tests verify state serialization and helper functions.
//! Integration tests (marked #[ignore]) test actual container operations.

use corten::config::{ContainerState, ContainerStatus, unix_timestamp};
use corten::container::is_process_alive;

#[test]
fn container_status_display() {
    assert_eq!(ContainerStatus::Created.to_string(), "created");
    assert_eq!(ContainerStatus::Running.to_string(), "running");
    assert_eq!(ContainerStatus::Stopped.to_string(), "stopped");
}

#[test]
fn container_state_serialization_roundtrip() {
    let state = ContainerState {
        status: ContainerStatus::Running,
        pid: Some(12345),
        created_at: 1710000000,
        started_at: Some(1710000001),
        finished_at: None,
        exit_code: None,
    };

    let json = serde_json::to_string(&state).unwrap();
    let deserialized: ContainerState = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.status, ContainerStatus::Running);
    assert_eq!(deserialized.pid, Some(12345));
    assert_eq!(deserialized.created_at, 1710000000);
    assert_eq!(deserialized.started_at, Some(1710000001));
    assert!(deserialized.finished_at.is_none());
    assert!(deserialized.exit_code.is_none());
}

#[test]
fn container_state_stopped_with_exit_code() {
    let state = ContainerState {
        status: ContainerStatus::Stopped,
        pid: Some(12345),
        created_at: 1710000000,
        started_at: Some(1710000001),
        finished_at: Some(1710000060),
        exit_code: Some(0),
    };

    let json = serde_json::to_string_pretty(&state).unwrap();
    assert!(json.contains("\"stopped\""));
    assert!(json.contains("12345"));
    assert!(json.contains("\"exit_code\": 0"));
}

#[test]
fn container_state_json_format() {
    let state = ContainerState {
        status: ContainerStatus::Created,
        pid: None,
        created_at: 1710000000,
        started_at: None,
        finished_at: None,
        exit_code: None,
    };

    let json = serde_json::to_string(&state).unwrap();
    // Status should be lowercase
    assert!(json.contains("\"created\""));
    // None fields should be null
    assert!(json.contains("null"));
}

#[test]
fn is_process_alive_detects_current_process() {
    let pid = std::process::id() as i32;
    assert!(is_process_alive(pid));
}

#[test]
fn is_process_alive_returns_false_for_invalid_pid() {
    // PID 0 refers to the kernel, not a regular process
    // A very high PID is unlikely to exist
    assert!(!is_process_alive(999_999_999));
}

#[test]
fn unix_timestamp_returns_reasonable_value() {
    let ts = unix_timestamp();
    // Should be after 2024-01-01 (1704067200) and before 2030-01-01 (1893456000)
    assert!(ts > 1704067200, "timestamp {ts} is too old");
    assert!(ts < 1893456000, "timestamp {ts} is too far in the future");
}

#[test]
fn container_status_equality() {
    assert_eq!(ContainerStatus::Created, ContainerStatus::Created);
    assert_eq!(ContainerStatus::Running, ContainerStatus::Running);
    assert_eq!(ContainerStatus::Stopped, ContainerStatus::Stopped);
    assert_ne!(ContainerStatus::Running, ContainerStatus::Stopped);
    assert_ne!(ContainerStatus::Created, ContainerStatus::Running);
}

#[test]
fn container_state_all_fields_populated() {
    let state = ContainerState {
        status: ContainerStatus::Stopped,
        pid: Some(42),
        created_at: 100,
        started_at: Some(101),
        finished_at: Some(200),
        exit_code: Some(137),
    };

    assert_eq!(state.exit_code, Some(137)); // 128 + 9 (SIGKILL)
    assert_eq!(state.finished_at.unwrap() - state.started_at.unwrap(), 99);
}

#[test]
fn find_container_fails_when_no_containers_dir() {
    // Set a nonexistent data dir
    // Safety: test runs single-threaded, no other code reads this var concurrently
    unsafe { std::env::set_var("CORTEN_DATA_DIR", "/tmp/corten-test-nonexistent") };
    let result = corten::container::find_container("anything");
    assert!(result.is_err());
    unsafe { std::env::remove_var("CORTEN_DATA_DIR") };
}
