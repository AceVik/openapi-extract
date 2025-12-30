# openapi-extract

[![Build Status](https://github.com/AceVik/openapi-extract/actions/workflows/ci.yml/badge.svg)](https://github.com/AceVik/openapi-extract/actions)

A production-grade, zero-runtime overhead OpenAPI generator for Rust. It extracts OpenAPI/Swagger definitions directly from your Rust code's comments (AST) and external files, merges them deeply, and outputs a single, validated `openapi.yaml` or `openapi.json`.

## Features

- **Mixed Inputs**: Seamlessly merges data from:
    - **Rust source code** (`.rs`) via AST parsing.
    - **YAML files** (`.yaml`, `.yml`).
    - **JSON files** (`.json`).
- **Smart References**: Use `$ModelName` in your comments to automatically link to `#/components/schemas/ModelName`. No quotes required in YAML!
- **Deep Merging**: Smarter than simple concatenation. It correctly merges nested paths, tags, and components.
- **Variable Substitution**: Automatically injects `{{CARGO_PKG_VERSION}}` and other variables.
- **Strict Validation**: Ensures a valid, single Root definition exists.

---

## Installation

### Method 1: CLI (Standalone)

Perfect for CI/CD pipelines or manual generation.

```bash
cargo install openapi-extract
# Or from source
cargo install --path .
```

### Method 2: Library (Build Script)

Ideal for keeping documentation in sync with every build.

Add to `Cargo.toml`:
```toml
[build-dependencies]
openapi-extract = "0.1.0"
```

Create `build.rs`:
```rust
fn main() {
    // Only rerun if source files change
    println!("cargo:rerun-if-changed=src");
    
    if let Err(e) = openapi_extract::Generator::new()
        .input("src")
        .output("docs/openapi.yaml")
        .generate() {
        // Log error but don't break build if docs fail (optional)
        eprintln!("Failed to generate OpenAPI: {}", e);
    }
}
```

---

## Configuration

Prority Order (Highest to Lowest):
1. **CLI Arguments** (`--input`, `--output`)
2. **Config File** (`--config path/to/config.toml`)
3. **Default Config** (`openapi.toml` in CWD)
4. **Cargo Metadata** (`[package.metadata.openapi-extract]` in `Cargo.toml`)

### Example `openapi.toml`
```toml
input = ["src", "libs/models"]
include = ["legacy/auth.yaml"]
output = "public/spec.json"
```

---

## Cookbook & Examples

### 1. Smart References (`$StructName`)
Stop typing `"$ref": "#/components/schemas/User"`. Just use `$User`.

**Rust Code:**
```rust
/// @openapi
/// components:
///   schemas:
///     User:
///       type: object
///       properties:
///         id: { type: integer }
struct User { id: i32 }

/// @openapi
/// paths:
///   /users/me:
///     get:
///       responses:
///         '200':
///           content:
///             application/json:
///               schema:
///                 $ref: $User  <-- LOOK HERE! No quotes needed.
fn me() {}
```

**Generated YAML:**
```yaml
paths:
  /users/me:
    get:
      responses:
        '200':
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/User"
```

### 2. JSON in Rust Comments
You can write JSON directly in Rust doc comments. Useful if you have existing JSON schemas.
**Note**: Multi-line JSON blocks must start with `{`.

```rust
/// {
///   "tags": [
///     { 
///       "name": "Billing", 
///       "description": "Billing endpoints" 
///     }
///   ]
/// }
pub mod billing {}
```

### 3. Security Schemes (Auth)
Define global security schemes in your Root Definition.

```rust
/// @openapi
/// openapi: 3.0.3
/// info:
///   title: Secure API
///   version: {{CARGO_PKG_VERSION}}
/// components:
///   securitySchemes:
///     BearerAuth:
///       type: http
///       scheme: bearer
///       bearerFormat: JWT
/// security:
///   - BearerAuth: []
struct ApiRoot;
```

### 4. Mixed Input (Migration Strategy)
You can validly mix a legacy `swagger.json` file with new Rust code.

```bash
openapi-extract \
  --input ./src \
  --include ./legacy-api.json \
  --output ./final-openapi.yaml
```

This allows you to migrate endpoints one by one from the legacy JSON file to Rust comments without breaking the final spec.

## License
MIT
