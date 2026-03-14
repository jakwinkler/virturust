//! Unit tests for port forwarding parsing.

use corten::config::parse_port;

#[test]
fn parse_port_basic() {
    let port = parse_port("8080:80").unwrap();
    assert_eq!(port.host_port, 8080);
    assert_eq!(port.container_port, 80);
    assert_eq!(port.host_ip, "0.0.0.0");
}

#[test]
fn parse_port_with_ip() {
    let port = parse_port("127.0.0.1:3000:3000").unwrap();
    assert_eq!(port.host_ip, "127.0.0.1");
    assert_eq!(port.host_port, 3000);
    assert_eq!(port.container_port, 3000);
}

#[test]
fn parse_port_same_ports() {
    let port = parse_port("443:443").unwrap();
    assert_eq!(port.host_port, 443);
    assert_eq!(port.container_port, 443);
}

#[test]
fn parse_port_high_ports() {
    let port = parse_port("49152:8080").unwrap();
    assert_eq!(port.host_port, 49152);
    assert_eq!(port.container_port, 8080);
}

#[test]
fn parse_port_invalid_format_fails() {
    assert!(parse_port("8080").is_err());
    assert!(parse_port("").is_err());
    assert!(parse_port(":").is_err());
    assert!(parse_port("abc:80").is_err());
    assert!(parse_port("8080:abc").is_err());
}

#[test]
fn parse_port_zero_fails() {
    assert!(parse_port("0:80").is_err());
    assert!(parse_port("8080:0").is_err());
}

#[test]
fn parse_port_too_high_fails() {
    assert!(parse_port("70000:80").is_err());
    assert!(parse_port("8080:70000").is_err());
}
