use corten::compose::{parse_compose_file, resolve_order};
use std::io::Write;

fn write_temp_yaml(content: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::with_suffix(".yml").unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file
}

#[test]
fn parse_minimal_compose() {
    let file = write_temp_yaml(r#"
services:
  web:
    image: alpine
"#);
    let comp = parse_compose_file(file.path()).unwrap();
    assert_eq!(comp.services.len(), 1);
    assert!(comp.services.contains_key("web"));
    assert_eq!(comp.services["web"].image, Some("alpine".to_string()));
}

#[test]
fn parse_multi_service_compose() {
    let file = write_temp_yaml(r#"
services:
  api:
    image: alpine
    ports:
      - "8080:80"
    depends_on:
      - db
  db:
    image: alpine
    environment:
      - DB_NAME=test
"#);
    let comp = parse_compose_file(file.path()).unwrap();
    assert_eq!(comp.services.len(), 2);
    assert_eq!(comp.services["api"].ports, vec!["8080:80"]);
    assert_eq!(comp.services["api"].depends_on, vec!["db"]);
}

#[test]
fn resolve_dependency_order() {
    let file = write_temp_yaml(r#"
services:
  worker:
    image: alpine
    depends_on:
      - api
  api:
    image: alpine
    depends_on:
      - db
  db:
    image: alpine
"#);
    let comp = parse_compose_file(file.path()).unwrap();
    let order = resolve_order(&comp).unwrap();

    let db_pos = order.iter().position(|s| s == "db").unwrap();
    let api_pos = order.iter().position(|s| s == "api").unwrap();
    let worker_pos = order.iter().position(|s| s == "worker").unwrap();

    assert!(db_pos < api_pos, "db should start before api");
    assert!(api_pos < worker_pos, "api should start before worker");
}

#[test]
fn detect_circular_dependency() {
    let file = write_temp_yaml(r#"
services:
  a:
    image: alpine
    depends_on:
      - b
  b:
    image: alpine
    depends_on:
      - a
"#);
    let comp = parse_compose_file(file.path()).unwrap();
    assert!(resolve_order(&comp).is_err());
}

#[test]
fn parse_with_deploy_resources() {
    let file = write_temp_yaml(r#"
services:
  web:
    image: alpine
    deploy:
      resources:
        limits:
          cpus: "0.5"
          memory: "256m"
"#);
    let comp = parse_compose_file(file.path()).unwrap();
    let deploy = comp.services["web"].deploy.as_ref().unwrap();
    let limits = deploy.resources.as_ref().unwrap().limits.as_ref().unwrap();
    assert_eq!(limits.cpus, Some("0.5".to_string()));
    assert_eq!(limits.memory, Some("256m".to_string()));
}

use clap::Parser;
use corten::cli::Cli;

#[test]
fn cli_compose_up() {
    let cli = Cli::parse_from(["corten", "compose", "up", "-d"]);
    match cli.command {
        corten::cli::Commands::Compose(args) => {
            match args.command {
                corten::cli::ComposeCommands::Up(up) => assert!(up.detach),
                _ => panic!("expected Up"),
            }
        }
        _ => panic!("expected Compose"),
    }
}

#[test]
fn cli_compose_down() {
    let cli = Cli::parse_from(["corten", "compose", "down"]);
    match cli.command {
        corten::cli::Commands::Compose(args) => {
            match args.command {
                corten::cli::ComposeCommands::Down => {},
                _ => panic!("expected Down"),
            }
        }
        _ => panic!("expected Compose"),
    }
}
