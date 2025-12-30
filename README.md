# openapi-extract

[![Build Status](https://github.com/AceVik/openapi-extract/actions/workflows/ci.yml/badge.svg)](https://github.com/AceVik/openapi-extract/actions)

A production-grade OpenAPI generator for Rust. Extracts, merges, and auto-wraps OpenAPI fragments from code comments.

## Features

- **Auto-Wrapping**: Define a struct's schema directly. The tool automatically places it under `components/schemas/StructName`.
- **Smart References**: Use `$User` to reference `#/components/schemas/User`.
- **Macros**:
  - `@extend Secured("admin")` → `x-openapi-extend` (auto-quoted).
  - `@insert Pagination` → Injects parameter references.
- **Mixed Inputs**: Merges `.rs`, `.yaml`, and `.json`.

## Installation

```bash
cargo install --path .
```

## Usage
### 1. Automatic Schema Definition (New!)
No need to write `components: schemas: ...`.

**Rust:**
```rust
/// @openapi
/// type: object
/// properties:
///   id: { type: integer }
struct User { id: i32 }
```

**Generates:**
```yaml
components:
  schemas:
    User:
      type: object
      properties:
        id: { type: integer }
```

### 2. Smart References & Generics
**Rust:**
```rust
/// @openapi
/// paths:
///   /users:
///     get:
///       responses:
///         '200':
///           content:
///             application/json:
///               schema:
///                 $ref: $Paginated<User>
fn list_users() {}
```

### Configuration (`openapi.toml`)
```toml
input = ["src"]
output = "openapi.yaml"
```
