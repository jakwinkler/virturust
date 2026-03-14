//! Tests for Corten.toml parsing and build configuration.

use corten::build::{detect_package_manager, parse_build_config, validate_build_config};
use std::io::Write;

fn write_temp_toml(content: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file
}

#[test]
fn parse_minimal_config() {
    let file = write_temp_toml(
        r#"
[base]
system = "alpine"
version = "3.19"
"#,
    );
    let config = parse_build_config(file.path()).unwrap();
    assert_eq!(config.base.system, "alpine");
    assert_eq!(config.base.version, "3.19");
    assert!(config.image.is_none());
    assert!(config.packages.is_none());
}

#[test]
fn parse_full_config() {
    let file = write_temp_toml(
        r#"
[image]
name = "my-app"
tag = "1.0"

[base]
system = "ubuntu"
version = "22.04"

[packages]
install = ["nginx", "curl"]

[files]
copy = [
    { src = "nginx.conf", dest = "/etc/nginx/nginx.conf" },
    { src = "app/", dest = "/var/www/html/", owner = "www-data" },
]

[env]
APP_ENV = "production"
DEBUG = "false"

[setup]
run = ["useradd -r appuser", "chmod 755 /var/www"]

[container]
command = ["nginx", "-g", "daemon off;"]
user = "appuser"
workdir = "/var/www/html"
expose = [80, 443]
"#,
    );
    let config = parse_build_config(file.path()).unwrap();

    let image = config.image.as_ref().unwrap();
    assert_eq!(image.name, "my-app");
    assert_eq!(image.tag, "1.0");

    assert_eq!(config.base.system, "ubuntu");

    let packages = config.packages.as_ref().unwrap();
    assert_eq!(packages.install, vec!["nginx", "curl"]);

    let files = config.files.as_ref().unwrap();
    assert_eq!(files.copy.len(), 2);
    assert_eq!(files.copy[0].src, "nginx.conf");
    assert_eq!(files.copy[1].owner, Some("www-data".to_string()));

    let env = config.env.as_ref().unwrap();
    assert_eq!(env["APP_ENV"], "production");

    let setup = config.setup.as_ref().unwrap();
    assert_eq!(setup.run.len(), 2);

    let container = config.container.as_ref().unwrap();
    assert_eq!(
        container.command,
        Some(vec![
            "nginx".to_string(),
            "-g".to_string(),
            "daemon off;".to_string()
        ])
    );
    assert_eq!(container.user, Some("appuser".to_string()));
    assert_eq!(container.expose, Some(vec![80, 443]));
}

#[test]
fn parse_example_files() {
    // Test that all example files parse successfully
    for name in &["nginx-php.toml", "postgres.toml", "simple-alpine.toml"] {
        let path = std::path::Path::new("examples").join(name);
        if path.exists() {
            let config = parse_build_config(&path);
            assert!(
                config.is_ok(),
                "failed to parse examples/{name}: {:?}",
                config.err()
            );
            let config = config.unwrap();
            validate_build_config(&config).unwrap();
        }
    }
}

#[test]
fn detect_package_manager_ubuntu() {
    assert_eq!(detect_package_manager("ubuntu"), "apt");
    assert_eq!(detect_package_manager("debian"), "apt");
}

#[test]
fn detect_package_manager_alpine() {
    assert_eq!(detect_package_manager("alpine"), "apk");
}

#[test]
fn detect_package_manager_fedora() {
    assert_eq!(detect_package_manager("fedora"), "dnf");
    assert_eq!(detect_package_manager("rhel"), "dnf");
}

#[test]
fn detect_package_manager_unknown() {
    assert_eq!(detect_package_manager("gentoo"), "unknown");
}

#[test]
fn validate_rejects_empty_dest() {
    let file = write_temp_toml(
        r#"
[base]
system = "alpine"
version = "3.19"

[files]
copy = [{ src = "test.txt", dest = "" }]
"#,
    );
    let config = parse_build_config(file.path()).unwrap();
    assert!(validate_build_config(&config).is_err());
}

#[test]
fn parse_invalid_toml_fails() {
    let file = write_temp_toml("this is not valid toml {{{}}}");
    assert!(parse_build_config(file.path()).is_err());
}

#[test]
fn parse_missing_base_fails() {
    let file = write_temp_toml(
        r#"
[image]
name = "test"
tag = "1.0"
"#,
    );
    assert!(parse_build_config(file.path()).is_err());
}
