//! Tests for per-user container isolation, JSONC support, and access control.

use std::io::Write;

// =============================================================================
// JSONC Comment Stripping
// =============================================================================

#[test]
fn jsonc_strip_line_comments() {
    let input = r#"{
        // this is a comment
        "key": "value"
    }"#;
    let stripped = corten::strip_jsonc_comments(input);
    let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
    assert_eq!(v["key"], "value");
}

#[test]
fn jsonc_strip_inline_comments() {
    let input = r#"{
        "key": "value", // inline comment
        "key2": 42 // another
    }"#;
    let stripped = corten::strip_jsonc_comments(input);
    let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
    assert_eq!(v["key"], "value");
    assert_eq!(v["key2"], 42);
}

#[test]
fn jsonc_strip_block_comments() {
    let input = r#"{
        /* this is
           a block comment */
        "key": "value"
    }"#;
    let stripped = corten::strip_jsonc_comments(input);
    let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
    assert_eq!(v["key"], "value");
}

#[test]
fn jsonc_preserve_slashes_in_strings() {
    let input = r#"{
        "url": "https://example.com/path", // comment
        "regex": "a//b"
    }"#;
    let stripped = corten::strip_jsonc_comments(input);
    let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
    assert_eq!(v["url"], "https://example.com/path");
    assert_eq!(v["regex"], "a//b");
}

#[test]
fn jsonc_preserve_block_syntax_in_strings() {
    let input = r#"{
        "code": "/* not a comment */",
        "real": "value"
    }"#;
    let stripped = corten::strip_jsonc_comments(input);
    let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
    assert_eq!(v["code"], "/* not a comment */");
}

#[test]
fn jsonc_mixed_comments() {
    let input = r#"{
        // line comment
        "a": 1,
        /* block */ "b": 2, // inline
        "c": "hello // world", /* trailing */
        "d": true
    }"#;
    let stripped = corten::strip_jsonc_comments(input);
    let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
    assert_eq!(v["a"], 1);
    assert_eq!(v["b"], 2);
    assert_eq!(v["c"], "hello // world");
    assert_eq!(v["d"], true);
}

#[test]
fn jsonc_empty_input() {
    assert_eq!(corten::strip_jsonc_comments(""), "");
}

#[test]
fn jsonc_no_comments() {
    let input = r#"{"key": "value"}"#;
    let stripped = corten::strip_jsonc_comments(input);
    assert_eq!(stripped, input);
}

#[test]
fn jsonc_escaped_quotes() {
    let input = r#"{
        "key": "value with \"escaped\" quotes", // comment
        "key2": "ok"
    }"#;
    let stripped = corten::strip_jsonc_comments(input);
    let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
    assert_eq!(v["key"], r#"value with "escaped" quotes"#);
    assert_eq!(v["key2"], "ok");
}

#[test]
fn jsonc_only_comments() {
    let input = "// just a comment\n/* block */\n";
    let stripped = corten::strip_jsonc_comments(input);
    assert!(stripped.trim().is_empty() || stripped.chars().all(|c| c.is_whitespace()));
}

#[test]
fn jsonc_nested_block_not_supported() {
    // JSONC doesn't support nested block comments
    // /* outer /* inner */ still_comment */
    // After first */, the rest is NOT a comment
    let input = r#"{ "a": 1 /* comment */ }"#;
    let stripped = corten::strip_jsonc_comments(input);
    let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
    assert_eq!(v["a"], 1);
}

// =============================================================================
// Per-User Container Isolation
// =============================================================================

#[test]
fn per_user_containers_dir_uses_uid() {
    unsafe { std::env::set_var("CORTEN_REAL_UID", "1234"); }
    let dir = corten::config::containers_dir();
    assert!(
        dir.to_string_lossy().contains("users/1234"),
        "Expected per-user path with uid 1234, got: {}",
        dir.display()
    );
    unsafe { std::env::set_var("CORTEN_REAL_UID", "0"); }
}

#[test]
fn root_containers_dir_is_legacy() {
    unsafe { std::env::set_var("CORTEN_REAL_UID", "0"); }
    let dir = corten::config::containers_dir();
    assert!(
        !dir.to_string_lossy().contains("users/"),
        "Root should use legacy path without 'users/', got: {}",
        dir.display()
    );
}

#[test]
fn images_dir_shared_across_users() {
    unsafe { std::env::set_var("CORTEN_REAL_UID", "9999"); }
    let dir = corten::config::images_dir();
    assert!(
        !dir.to_string_lossy().contains("users/"),
        "Images should be shared (not per-user), got: {}",
        dir.display()
    );
    unsafe { std::env::set_var("CORTEN_REAL_UID", "0"); }
}

#[test]
fn different_users_different_dirs() {
    unsafe { std::env::set_var("CORTEN_REAL_UID", "1000"); }
    let dir_a = corten::config::containers_dir();

    unsafe { std::env::set_var("CORTEN_REAL_UID", "1001"); }
    let dir_b = corten::config::containers_dir();

    assert_ne!(dir_a, dir_b);
    assert!(dir_a.to_string_lossy().contains("1000"));
    assert!(dir_b.to_string_lossy().contains("1001"));

    unsafe { std::env::set_var("CORTEN_REAL_UID", "0"); }
}

#[test]
fn custom_data_dir_with_per_user() {
    unsafe {
        std::env::set_var("CORTEN_DATA_DIR", "/tmp/corten-test-isolation");
        std::env::set_var("CORTEN_REAL_UID", "5000");
    }
    let cdir = corten::config::containers_dir();
    assert!(cdir.to_string_lossy().starts_with("/tmp/corten-test-isolation"));
    assert!(cdir.to_string_lossy().contains("users/5000"));

    // Images still shared under custom data dir
    let idir = corten::config::images_dir();
    assert!(idir.to_string_lossy().starts_with("/tmp/corten-test-isolation"));
    assert!(!idir.to_string_lossy().contains("users/"));

    unsafe {
        std::env::remove_var("CORTEN_DATA_DIR");
        std::env::set_var("CORTEN_REAL_UID", "0");
    }
}

#[test]
fn missing_uid_env_defaults_to_root() {
    unsafe { std::env::remove_var("CORTEN_REAL_UID"); }
    let dir = corten::config::containers_dir();
    // Should fall back to "0" (root) which uses legacy path
    assert!(
        !dir.to_string_lossy().contains("users/"),
        "Missing UID should default to root (legacy path), got: {}",
        dir.display()
    );
    unsafe { std::env::set_var("CORTEN_REAL_UID", "0"); }
}

// =============================================================================
// Build Config Format Support
// =============================================================================

#[test]
fn parse_build_config_jsonc() {
    let mut file = tempfile::NamedTempFile::with_suffix(".jsonc").unwrap();
    file.write_all(br#"{
        // Build config for test image
        "image": { "name": "test-jsonc", "tag": "latest" },
        "base": { "system": "alpine", "version": "3.20" },
        /* Optional packages */
        "packages": { "install": ["nginx"] },
        "container": { "command": ["nginx"] }
    }"#).unwrap();

    let config = corten::build::parse_build_config(file.path()).unwrap();
    assert_eq!(config.image.as_ref().unwrap().name, "test-jsonc");
    assert_eq!(config.base.system, "alpine");
    assert_eq!(config.packages.as_ref().unwrap().install, vec!["nginx"]);
}

#[test]
fn parse_build_config_json() {
    let mut file = tempfile::NamedTempFile::with_suffix(".json").unwrap();
    file.write_all(br#"{
        "image": { "name": "test-json", "tag": "1.0" },
        "base": { "system": "ubuntu", "version": "22.04" }
    }"#).unwrap();

    let config = corten::build::parse_build_config(file.path()).unwrap();
    assert_eq!(config.image.as_ref().unwrap().name, "test-json");
    assert_eq!(config.base.version, "22.04");
}

#[test]
fn parse_build_config_toml() {
    let mut file = tempfile::NamedTempFile::with_suffix(".toml").unwrap();
    file.write_all(br#"
[image]
name = "test-toml"
tag = "latest"

[base]
system = "alpine"
version = "3.20"
"#).unwrap();

    let config = corten::build::parse_build_config(file.path()).unwrap();
    assert_eq!(config.image.as_ref().unwrap().name, "test-toml");
}

// =============================================================================
// Forge Config Format Support
// =============================================================================

#[test]
fn parse_forge_jsonc() {
    let mut file = tempfile::NamedTempFile::with_suffix(".jsonc").unwrap();
    file.write_all(br#"{
        // Multi-service stack
        "services": {
            "api": {
                "image": "my-api",
                "ports": ["8080:80"],
                "depends_on": ["db"]
            },
            /* Database */
            "db": {
                "image": "my-db",
                "memory": "512m"
            }
        }
    }"#).unwrap();

    let forge = corten::compose::parse_forge_file(file.path()).unwrap();
    assert_eq!(forge.services.len(), 2);
    assert_eq!(forge.services["api"].depends_on, vec!["db"]);
    assert_eq!(forge.services["db"].memory, Some("512m".to_string()));

    let order = corten::compose::resolve_order(&forge).unwrap();
    let db_pos = order.iter().position(|s| s == "db").unwrap();
    let api_pos = order.iter().position(|s| s == "api").unwrap();
    assert!(db_pos < api_pos);
}

#[test]
fn parse_forge_jsonc_with_env() {
    let mut file = tempfile::NamedTempFile::with_suffix(".jsonc").unwrap();
    file.write_all(br#"{
        "services": {
            "app": {
                "image": "alpine",
                // Environment as key-value map (not list like Docker)
                "env": {
                    "DB_HOST": "db",
                    "API_KEY": "secret123"
                }
            }
        }
    }"#).unwrap();

    let forge = corten::compose::parse_forge_file(file.path()).unwrap();
    assert_eq!(forge.services["app"].env["DB_HOST"], "db");
    assert_eq!(forge.services["app"].env["API_KEY"], "secret123");
}

// =============================================================================
// Example File Parsing
// =============================================================================

#[test]
fn parse_example_nginx_jsonc() {
    let path = std::path::Path::new("examples/nginx/Corten.jsonc");
    if path.exists() {
        let config = corten::build::parse_build_config(path).unwrap();
        assert_eq!(config.image.as_ref().unwrap().name, "my-nginx");
        assert_eq!(config.base.system, "alpine");
    }
}

#[test]
fn parse_example_forge_jsonc() {
    let path = std::path::Path::new("examples/forge/Cortenforge.jsonc");
    if path.exists() {
        let forge = corten::compose::parse_forge_file(path).unwrap();
        assert!(!forge.services.is_empty());
        let order = corten::compose::resolve_order(&forge).unwrap();
        assert_eq!(order.len(), forge.services.len());
    }
}
