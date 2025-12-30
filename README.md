# openapi-extract

[![CI](https://github.com/viktor/openapi-extract/actions/workflows/ci.yml/badge.svg)](https://github.com/viktor/openapi-extract/actions/workflows/ci.yml)

`openapi-extract` is a production-grade Rust tool that treats your code as the source of truth for your API documentation. It extracts OpenAPI/Swagger definitions directly from Rust documentation comments (`///`), recursively merges them, and generates a validated `openapi.yaml` or `openapi.json` file.

## Features

- **AST Parsing**: Reliable parsing using `syn`, avoiding regex fragility.
- **Deep Merging**: Smartly merges partial definitions (e.g., adds paths to the root, appends tags).
- **Multi-Format**: Supports YAML and JSON input fragments. output format inferred by extension.
- **Variable Substitution**: Automatically injects `{{CARGO_PKG_VERSION}}` from your environment.
- **Configuration Layers**: Flexible config via CLI, `openapi.toml`, or `Cargo.toml`.

## Installation

### From Source

```bash
cargo install --path .
```

### As a Library (Build Script)

Add to `Cargo.toml`:

```toml
[build-dependencies]
openapi-extract = { version = "0.2.0", default-features = false }
```

## Usage

### CLI

```bash
# Basic usage
openapi-extract --input ./src --output docs/openapi.yaml

# Multiple inputs and explicit include
openapi-extract -i ./src -i ./libs/common --include ./legacy/swagger.json -o openapi.json
```

### Configuration Priorities

Settings are loaded in the following order (highest priority first):

1.  **CLI Arguments**: (`--input`, `--output`)
2.  **Config File**: (`--config my-config.toml`)
3.  **Default Config**: (`openapi.toml` in current directory)
4.  **Cargo Metadata**: (`[package.metadata.openapi-extract]` in `Cargo.toml`)

#### Example `openapi.toml`

```toml
input = ["src", "crates/api"]
output = "dist/openapi.yaml"
include = ["static/base.yaml"]
```

#### Example `Cargo.toml`

```toml
[package.metadata.openapi-extract]
input = ["src"]
output = "openapi.yaml"
```

## Writing Documentation

You can write OpenAPI fragments in YAML (default) or JSON.

### Root Definition (Required Just Once)

```rust
/// @openapi
/// openapi: 3.0.3
/// info:
///   title: My Awesome API
///   version: {{CARGO_PKG_VERSION}}
struct ApiRoot;
```

### Path Fragments

```rust
/// @openapi
/// paths:
///   /users:
///     get:
///       summary: List users
///       tags:
///         - Users
fn list_users() {}
```

### JSON Style

```rust
/// {
///   "tags": [
///     { "name": "Users", "description": "User management" }
///   ]
/// }
mod users {}
```

## License

MIT
