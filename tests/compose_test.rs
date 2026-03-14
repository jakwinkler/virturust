//! Tests for Corten Forge (multi-container orchestration from TOML).

use corten::compose::{parse_forge_file, resolve_order};
use std::io::Write;

fn write_temp_toml(content: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::with_suffix(".toml").unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file
}

fn write_temp_json(content: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::with_suffix(".json").unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file
}

#[test]
fn parse_minimal_forge() {
    let file = write_temp_toml(r#"
[services.web]
image = "alpine"
"#);
    let forge = parse_forge_file(file.path()).unwrap();
    assert_eq!(forge.services.len(), 1);
    assert!(forge.services.contains_key("web"));
    assert_eq!(forge.services["web"].image, Some("alpine".to_string()));
}

#[test]
fn parse_multi_service_forge() {
    let file = write_temp_toml(r#"
[services.api]
image = "alpine"
ports = ["8080:80"]
depends_on = ["db"]

[services.db]
image = "alpine"

[services.db.env]
DB_NAME = "test"
"#);
    let forge = parse_forge_file(file.path()).unwrap();
    assert_eq!(forge.services.len(), 2);
    assert_eq!(forge.services["api"].ports, vec!["8080:80"]);
    assert_eq!(forge.services["api"].depends_on, vec!["db"]);
    assert_eq!(forge.services["db"].env["DB_NAME"], "test");
}

#[test]
fn parse_flat_resources() {
    let file = write_temp_toml(r#"
[services.web]
image = "alpine"
memory = "256m"
cpus = "0.5"
"#);
    let forge = parse_forge_file(file.path()).unwrap();
    assert_eq!(forge.services["web"].memory, Some("256m".to_string()));
    assert_eq!(forge.services["web"].cpus, Some("0.5".to_string()));
}

#[test]
fn resolve_dependency_order() {
    let file = write_temp_toml(r#"
[services.worker]
image = "alpine"
depends_on = ["api"]

[services.api]
image = "alpine"
depends_on = ["db"]

[services.db]
image = "alpine"
"#);
    let forge = parse_forge_file(file.path()).unwrap();
    let order = resolve_order(&forge).unwrap();

    let db_pos = order.iter().position(|s| s == "db").unwrap();
    let api_pos = order.iter().position(|s| s == "api").unwrap();
    let worker_pos = order.iter().position(|s| s == "worker").unwrap();

    assert!(db_pos < api_pos, "db should start before api");
    assert!(api_pos < worker_pos, "api should start before worker");
}

#[test]
fn detect_circular_dependency() {
    let file = write_temp_toml(r#"
[services.a]
image = "alpine"
depends_on = ["b"]

[services.b]
image = "alpine"
depends_on = ["a"]
"#);
    let forge = parse_forge_file(file.path()).unwrap();
    assert!(resolve_order(&forge).is_err());
}

#[test]
fn parse_json_format() {
    let file = write_temp_json(r#"{
        "services": {
            "web": {
                "image": "alpine",
                "ports": ["8080:80"],
                "memory": "128m"
            }
        }
    }"#);
    let forge = parse_forge_file(file.path()).unwrap();
    assert_eq!(forge.services["web"].image, Some("alpine".to_string()));
    assert_eq!(forge.services["web"].memory, Some("128m".to_string()));
}

#[test]
fn parse_example_forge_file() {
    let path = std::path::Path::new("examples/forge/Cortenforge.toml");
    if path.exists() {
        let forge = parse_forge_file(path).unwrap();
        assert!(!forge.services.is_empty());
        let order = resolve_order(&forge).unwrap();
        assert_eq!(order.len(), forge.services.len());
    }
}

use clap::Parser;
use corten::cli::Cli;

#[test]
fn cli_forge_up() {
    let cli = Cli::parse_from(["corten", "forge", "up", "-d"]);
    match cli.command {
        corten::cli::Commands::Forge(args) => match args.command {
            corten::cli::ComposeCommands::Up(up) => assert!(up.detach),
            _ => panic!("expected Up"),
        },
        _ => panic!("expected Forge"),
    }
}

#[test]
fn cli_forge_down() {
    let cli = Cli::parse_from(["corten", "forge", "down"]);
    match cli.command {
        corten::cli::Commands::Forge(args) => match args.command {
            corten::cli::ComposeCommands::Down => {}
            _ => panic!("expected Down"),
        },
        _ => panic!("expected Forge"),
    }
}
