//! Tests for configuration parsing and validation.

use corten::config::{parse_image_ref, parse_memory};

// =============================================================================
// parse_memory tests
// =============================================================================

#[test]
fn parse_memory_megabytes_lowercase() {
    assert_eq!(parse_memory("256m").unwrap(), 256 * 1024 * 1024);
}

#[test]
fn parse_memory_megabytes_uppercase() {
    assert_eq!(parse_memory("256M").unwrap(), 256 * 1024 * 1024);
}

#[test]
fn parse_memory_gigabytes_lowercase() {
    assert_eq!(parse_memory("1g").unwrap(), 1024 * 1024 * 1024);
}

#[test]
fn parse_memory_gigabytes_uppercase() {
    assert_eq!(parse_memory("2G").unwrap(), 2 * 1024 * 1024 * 1024);
}

#[test]
fn parse_memory_kilobytes_lowercase() {
    assert_eq!(parse_memory("512k").unwrap(), 512 * 1024);
}

#[test]
fn parse_memory_kilobytes_uppercase() {
    assert_eq!(parse_memory("512K").unwrap(), 512 * 1024);
}

#[test]
fn parse_memory_raw_bytes() {
    assert_eq!(parse_memory("1048576").unwrap(), 1_048_576);
}

#[test]
fn parse_memory_one_byte() {
    assert_eq!(parse_memory("1").unwrap(), 1);
}

#[test]
fn parse_memory_zero() {
    assert_eq!(parse_memory("0").unwrap(), 0);
}

#[test]
fn parse_memory_large_value() {
    // 16 GiB
    assert_eq!(parse_memory("16g").unwrap(), 16 * 1024 * 1024 * 1024);
}

#[test]
fn parse_memory_with_whitespace() {
    assert_eq!(parse_memory("  256m  ").unwrap(), 256 * 1024 * 1024);
}

#[test]
fn parse_memory_empty_string_fails() {
    assert!(parse_memory("").is_err());
}

#[test]
fn parse_memory_only_suffix_fails() {
    assert!(parse_memory("m").is_err());
    assert!(parse_memory("g").is_err());
    assert!(parse_memory("k").is_err());
}

#[test]
fn parse_memory_invalid_number_fails() {
    assert!(parse_memory("abc").is_err());
    assert!(parse_memory("12.5m").is_err());
    assert!(parse_memory("-1m").is_err());
}

#[test]
fn parse_memory_only_whitespace_fails() {
    assert!(parse_memory("   ").is_err());
}

#[test]
fn parse_memory_overflow_fails() {
    // u64::MAX as kilobytes would overflow
    assert!(parse_memory("99999999999999999999k").is_err());
}

// =============================================================================
// parse_image_ref tests
// =============================================================================

#[test]
fn parse_image_ref_name_only() {
    assert_eq!(parse_image_ref("alpine"), ("alpine", "latest"));
}

#[test]
fn parse_image_ref_with_tag() {
    assert_eq!(parse_image_ref("ubuntu:22.04"), ("ubuntu", "22.04"));
}

#[test]
fn parse_image_ref_with_latest() {
    assert_eq!(parse_image_ref("debian:latest"), ("debian", "latest"));
}

#[test]
fn parse_image_ref_bookworm() {
    assert_eq!(parse_image_ref("debian:bookworm"), ("debian", "bookworm"));
}

#[test]
fn parse_image_ref_with_org() {
    assert_eq!(
        parse_image_ref("myorg/myimage:v1"),
        ("myorg/myimage", "v1")
    );
}

#[test]
fn parse_image_ref_with_org_no_tag() {
    assert_eq!(
        parse_image_ref("myorg/myimage"),
        ("myorg/myimage", "latest")
    );
}

#[test]
fn parse_image_ref_numeric_tag() {
    assert_eq!(parse_image_ref("node:20"), ("node", "20"));
}

#[test]
fn parse_image_ref_complex_tag() {
    assert_eq!(
        parse_image_ref("python:3.12-slim-bookworm"),
        ("python", "3.12-slim-bookworm")
    );
}
