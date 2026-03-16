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

/// Strip JSONC comments (// and /* */) from a string.
/// Supports JSON with Comments — the format VS Code uses.
pub fn strip_jsonc_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escape_next = false;

    while let Some(c) = chars.next() {
        if escape_next {
            result.push(c);
            escape_next = false;
            continue;
        }

        if in_string {
            result.push(c);
            if c == '\\' {
                escape_next = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        match c {
            '"' => {
                in_string = true;
                result.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                // Line comment — skip until newline
                chars.next(); // consume second /
                while let Some(&next) = chars.peek() {
                    if next == '\n' { break; }
                    chars.next();
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                // Block comment — skip until */
                chars.next(); // consume *
                loop {
                    match chars.next() {
                        Some('*') if chars.peek() == Some(&'/') => {
                            chars.next(); // consume /
                            break;
                        }
                        Some(_) => continue,
                        None => break,
                    }
                }
            }
            _ => result.push(c),
        }
    }
    result
}
