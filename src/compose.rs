//! Forge — multi-container orchestration from TOML.
//!
//! Corten Forge lets you define and run multi-container applications
//! using `Cortenforge.toml` — the same TOML format used for image
//! builds, no YAML needed.
//!
//! ## Example
//!
//! ```toml
//! [services.api]
//! image = "my-app"
//! ports = ["8080:80"]
//! depends_on = ["db"]
//! memory = "256m"
//!
//! [services.api.env]
//! DB_HOST = "db"
//!
//! [services.db]
//! image = "my-mysql"
//! memory = "512m"
//! ```
//!
//! ```bash
//! corten forge up      # start in dependency order
//! corten forge ps      # list services
//! corten forge logs    # view output
//! corten forge down    # stop and clean up
//! ```

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Top-level Cortenforge.toml structure.
#[derive(Debug, Deserialize)]
pub struct ForgeFile {
    /// Service definitions
    pub services: HashMap<String, Service>,
}

/// A single service definition.
///
/// Flat config — no deep nesting like Docker Compose's
/// `deploy.resources.limits.memory`. Just `memory = "256m"`.
#[derive(Debug, Deserialize)]
pub struct Service {
    /// Image to use (e.g., "alpine", "my-nginx")
    pub image: Option<String>,

    /// Build context (path to Corten.toml) — build instead of pull
    pub build: Option<String>,

    /// Command override
    pub command: Option<Vec<String>>,

    /// Entrypoint override
    pub entrypoint: Option<String>,

    /// Environment variables as key-value map
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Port mappings (e.g., ["8080:80", "443:443"])
    #[serde(default)]
    pub ports: Vec<String>,

    /// Volume mounts (e.g., ["/host:/container", "/data:/mnt:ro"])
    #[serde(default)]
    pub volumes: Vec<String>,

    /// Network to join (named network)
    pub network: Option<String>,

    /// Service dependencies — started before this service
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Restart policy: "no", "always", "on-failure:N"
    pub restart: Option<String>,

    /// Container name override
    pub container_name: Option<String>,

    /// Memory limit (e.g., "256m", "1g") — flat, not nested
    pub memory: Option<String>,

    /// CPU limit (e.g., "0.5", "2.0") — flat, not nested
    pub cpus: Option<String>,

    /// Hostname
    pub hostname: Option<String>,

    /// Working directory
    pub working_dir: Option<String>,

    /// User
    pub user: Option<String>,

    /// Privileged mode
    #[serde(default)]
    pub privileged: bool,

    /// Read-only root filesystem
    #[serde(default)]
    pub read_only: bool,
}

/// Parse a forge file (auto-detected by extension).
///
/// Supported formats:
/// - `.toml` — TOML (default, recommended)
/// - `.json` — JSON
/// - `.jsonc` — JSON with Comments
pub fn parse_forge_file(path: &Path) -> Result<ForgeFile> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("toml");

    let forge: ForgeFile = match ext {
        "json" => serde_json::from_str(&content)
            .with_context(|| format!("failed to parse JSON: {}", path.display()))?,
        "jsonc" => {
            let stripped = crate::strip_jsonc_comments(&content);
            serde_json::from_str(&stripped)
                .with_context(|| format!("failed to parse JSONC: {}", path.display()))?
        }
        _ => toml::from_str(&content)
            .with_context(|| format!("failed to parse TOML: {}", path.display()))?,
    };

    Ok(forge)
}

/// Resolve service startup order based on depends_on.
/// Returns services in dependency order (dependencies first).
pub fn resolve_order(forge: &ForgeFile) -> Result<Vec<String>> {
    let mut order = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut visiting = std::collections::HashSet::new();

    for name in forge.services.keys() {
        topo_sort(name, &forge.services, &mut order, &mut visited, &mut visiting)?;
    }

    Ok(order)
}

fn topo_sort(
    name: &str,
    services: &HashMap<String, Service>,
    order: &mut Vec<String>,
    visited: &mut std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Result<()> {
    if visited.contains(name) {
        return Ok(());
    }
    if visiting.contains(name) {
        return Err(anyhow!("circular dependency detected involving '{name}'"));
    }

    visiting.insert(name.to_string());

    if let Some(service) = services.get(name) {
        for dep in &service.depends_on {
            if !services.contains_key(dep.as_str()) {
                return Err(anyhow!(
                    "service '{name}' depends on unknown service '{dep}'"
                ));
            }
            topo_sort(dep, services, order, visited, visiting)?;
        }
    }

    visiting.remove(name);
    visited.insert(name.to_string());
    order.push(name.to_string());
    Ok(())
}

/// Print a summary of the forge file.
pub fn print_forge_summary(forge: &ForgeFile) {
    println!("Services ({}):", forge.services.len());
    for (name, svc) in &forge.services {
        let image = svc.image.as_deref().unwrap_or("(build)");
        let ports = if svc.ports.is_empty() {
            String::new()
        } else {
            format!(" ports: {}", svc.ports.join(", "))
        };
        let deps = if svc.depends_on.is_empty() {
            String::new()
        } else {
            format!(" depends_on: {}", svc.depends_on.join(", "))
        };
        let mem = svc
            .memory
            .as_deref()
            .map(|m| format!(" mem: {m}"))
            .unwrap_or_default();
        println!("  {name}: {image}{ports}{deps}{mem}");
    }
}
