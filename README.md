# oas-forge ⚒️
> The zero-runtime OpenAPI compiler for Rust.

[![Crates.io](https://img.shields.io/crates/v/oas-forge.svg)](https://crates.io/crates/oas-forge)
[![Documentation](https://docs.rs/oas-forge/badge.svg)](https://docs.rs/oas-forge)
[![License](https://img.shields.io/crates/l/oas-forge.svg)](https://github.com/AceVik/oas-forge/blob/main/LICENSE)

**oas-forge** (formerly `openapi-extract`) is a production-grade tool that extracts, merges, and compiles OpenAPI 3.1 specifications directly from your Rust code comments (AST) and external files.

## 🚀 Features
- **Zero Runtime Overhead**: Runs at compile time or in CI/CD.
- **Source-Mapped Error Reporting**: Precise error messages pointing to file and line.
- **Auto-Enum Extraction**: Automatically converts Rust Enums to OpenAPI String Enums.
- **Smart Merging**: Recursively merges partial OpenAPI fragments (YAML/JSON).

## 📦 Installation
```toml
[dev-dependencies]
oas-forge = "0.3.1"
```

## 🛠 Usage

### Configuration (`Cargo.toml`)
```toml
[package.metadata.oas-forge]
input = ["src", "controllers"]
output = "openapi.yaml"
```

### Auto-Enum Extraction
```rust
/// @openapi
enum Role { Admin, User }
```
Generates:
```yaml
components:
  schemas:
    Role:
      type: string
      enum:
        - Admin
        - User
```
