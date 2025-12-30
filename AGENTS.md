# AGENTS.md

This document is intended for AI Agents (Cursor, Copilot, Windsurf) to understand the `openapi-extract` crate architecture without parsing the entire codebase.

## Overview

`openapi-extract` scans Rust source code and other files to generate a unified OpenAPI v3 definition. It acts as both a CLI tool and a library for `build.rs` integration.

## Architecture

- **`src/lib.rs`**: Exposes the `Generator` struct, the main public API.
- **`src/config.rs`**: Handles configuration loading from multiple sources (CLI -> File -> Cargo.toml).
- **`src/scanner.rs`**: Recursively walks directories. Handles variable substitution (`{{VAR}}`) and detects file types (`.rs`, `.json`, `.yaml`).
- **`src/visitor.rs`**: Uses `syn` to parse Rust AST. Extracts `#[doc]` attributes matching `@openapi` or starting with `{`.
- **`src/merger.rs`**: Contains the `merge_openapi` logic. Deep merges maps, appends arrays, validates exactly one `Root` (openapi+info).

## Public API (`Generator`)

```rust
pub struct Generator { ... }

impl Generator {
    /// Initialize default generator
    pub fn new() -> Self;

    /// Apply configuration object
    pub fn with_config(self, config: Config) -> Self;

    /// Add an input directory to scan (recursive)
    pub fn input<P: Into<PathBuf>>(self, path: P) -> Self;

    /// Add a specific file (JSON/YAML) to include directly
    pub fn include<P: Into<PathBuf>>(self, path: P) -> Self;

    /// Set output path. Extension determines format (.json vs .yaml)
    pub fn output<P: Into<PathBuf>>(self, path: P) -> Self;

    /// Execute the generation
    pub fn generate(self) -> Result<()>;
}
```

## Configuration (`Config`)

The `Config` struct is serializable (serde) and derive-able (clap).

```rust
pub struct Config {
    pub input: Option<Vec<PathBuf>>,
    pub include: Option<Vec<PathBuf>>,
    pub output: Option<PathBuf>,
}
```

## Usage Pattern (Code Generation)

If you are an AI generating code to use this tool in `build.rs`:

```rust
// build.rs
fn main() {
    println!("cargo:rerun-if-changed=src");
    if let Err(e) = openapi_extract::Generator::new()
        .input("src")
        .output("openapi.yaml")
        .generate() {
        eprintln!("Error generating OpenAPI: {}", e);
        // Do not panic if you want non-blocking failure
    }
}
```
