//! Compose — multi-container orchestration from YAML.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Top-level compose file structure.
#[derive(Debug, Deserialize)]
pub struct ComposeFile {
    /// Service definitions
    pub services: HashMap<String, Service>,
    /// Network definitions (optional)
    #[serde(default)]
    pub networks: HashMap<String, NetworkDef>,
}

/// A single service definition.
#[derive(Debug, Deserialize)]
pub struct Service {
    /// Image to use
    pub image: Option<String>,
    /// Build context (path to Corten.toml)
    pub build: Option<String>,
    /// Command override
    pub command: Option<Vec<String>>,
    /// Entrypoint override
    pub entrypoint: Option<String>,
    /// Environment variables
    #[serde(default)]
    pub environment: Vec<String>,
    /// Port mappings
    #[serde(default)]
    pub ports: Vec<String>,
    /// Volume mounts
    #[serde(default)]
    pub volumes: Vec<String>,
    /// Network to join
    #[serde(default)]
    pub networks: Vec<String>,
    /// Service dependencies
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Restart policy
    pub restart: Option<String>,
    /// Container name override
    pub container_name: Option<String>,
    /// Resource limits
    pub deploy: Option<Deploy>,
    /// Hostname
    pub hostname: Option<String>,
    /// Working directory
    pub working_dir: Option<String>,
    /// User
    pub user: Option<String>,
    /// Privileged mode
    #[serde(default)]
    pub privileged: bool,
    /// Read-only root
    #[serde(default)]
    pub read_only: bool,
}

/// Deploy/resource configuration.
#[derive(Debug, Deserialize)]
pub struct Deploy {
    pub resources: Option<Resources>,
}

#[derive(Debug, Deserialize)]
pub struct Resources {
    pub limits: Option<ResourceLimits>,
}

#[derive(Debug, Deserialize)]
pub struct ResourceLimits {
    pub cpus: Option<String>,
    pub memory: Option<String>,
}

/// Network definition in compose file.
#[derive(Debug, Default, Deserialize)]
pub struct NetworkDef {
    pub driver: Option<String>,
}

/// Parse a compose file.
pub fn parse_compose_file(path: &Path) -> Result<ComposeFile> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let compose: ComposeFile = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(compose)
}

/// Resolve service startup order based on depends_on.
/// Returns services in dependency order (dependencies first).
pub fn resolve_order(compose: &ComposeFile) -> Result<Vec<String>> {
    let mut order = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut visiting = std::collections::HashSet::new();

    for name in compose.services.keys() {
        topo_sort(name, &compose.services, &mut order, &mut visited, &mut visiting)?;
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
    if visited.contains(name) { return Ok(()); }
    if visiting.contains(name) {
        return Err(anyhow!("circular dependency detected involving '{name}'"));
    }

    visiting.insert(name.to_string());

    if let Some(service) = services.get(name) {
        for dep in &service.depends_on {
            if !services.contains_key(dep.as_str()) {
                return Err(anyhow!("service '{name}' depends on unknown service '{dep}'"));
            }
            topo_sort(dep, services, order, visited, visiting)?;
        }
    }

    visiting.remove(name);
    visited.insert(name.to_string());
    order.push(name.to_string());
    Ok(())
}

/// Print a summary of the compose file.
pub fn print_compose_summary(compose: &ComposeFile) {
    println!("Services ({}):", compose.services.len());
    for (name, svc) in &compose.services {
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
        println!("  {name}: {image}{ports}{deps}");
    }
    if !compose.networks.is_empty() {
        println!("Networks: {}", compose.networks.keys().cloned().collect::<Vec<_>>().join(", "));
    }
}
