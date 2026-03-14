//! # Corten
//!
//! A lightweight, high-performance container runtime written in Rust.
//!
//! Corten provides Docker-like containerization using Linux kernel primitives:
//!
//! - **Namespaces** for process, mount, network, UTS, and IPC isolation
//! - **cgroups v2** for memory, CPU, and process count limits
//! - **pivot_root** for complete filesystem isolation
//! - **OCI images** pulled directly from Docker Hub
//!
//! ## Crate structure
//!
//! | Module         | Purpose                                            |
//! |----------------|----------------------------------------------------|
//! | [`cgroup`]     | cgroups v2 resource limit enforcement              |
//! | [`cli`]        | Command-line argument parsing                      |
//! | [`config`]     | Configuration types and parsing utilities           |
//! | [`container`]  | Container lifecycle management                     |
//! | [`filesystem`] | Mount setup and `pivot_root` isolation             |
//! | [`image`]      | OCI image pulling from Docker Hub                  |
//! | [`namespace`]  | Linux namespace creation via `clone()`             |
//! | [`build`]      | Corten.toml build file parser                      |
//! | [`network`]    | Network namespace setup                            |

pub mod build;
pub mod compose;
pub mod cgroup;
pub mod cli;
pub mod config;
pub mod container;
pub mod filesystem;
pub mod image;
pub mod namespace;
pub mod network;
pub mod security;
