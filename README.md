# openapi-extract v0.2.0

[![Build Status](https://github.com/AceVik/openapi-extract/actions/workflows/ci.yml/badge.svg)](https://github.com/AceVik/openapi-extract/actions)
[![Latest Release](https://img.shields.io/github/v/release/AceVik/openapi-extract)](https://github.com/AceVik/openapi-extract/releases/latest)

A production-grade, zero-runtime overhead OpenAPI generator for Rust. It implements a powerful **Pre-Processor** and **Type System** to enable "Docs-as-Code" with Fragments, Mixins, and Generics.

## Features (v0.2.0)

- **Fragments & Mixins**: Define reusable YAML blocks (`@openapi-fragment`) and inject them (`@insert`, `@extend`).
- **Generics**: Define schema blueprints (`@openapi<T>`) and instantiate them (`$Page<User>`).
- **Smart References**: Use `$ModelName` to link to schemas.
- **Runtime Variables**: Use `$$VAR` for runtime string replacement (e.g. OIDC URLs).
- **Mixed Inputs**: Merges `.rs` comments, `.yaml`, and `.json` files.

---

## Installation & Setup

### 1. Dependencies (Cargo.toml)

```toml
[build-dependencies]
openapi-extract = { git = "https://github.com/AceVik/openapi-extract", tag = "v0.2.0" }
```

### 2. Build Script (`build.rs`) - **REQUIRED**

You **must** use a build script to generate the spec at compile time.

```rust
// build.rs
use openapi_extract::config::Config;
use openapi_extract::Generator;

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    
    // Load config from openapi.toml or defaults
    let config = Config::load();
    
    if let Err(e) = Generator::new().with_config(config).generate() {
         eprintln!("Warning: Failed to generate OpenAPI: {}", e);
    }
}
```

---

## Feature Guide

### 1. Fragments & Mixins

Don't repeat yourself. Define fragments for common headers, responses, or error models.

**Definition:**
```rust
//! @openapi-fragment CommonHeaders(name)
//! parameters:
//!   - in: header
//!     name: {{arg0}}
//!     schema: { type: string }
```

**Usage (Insert):** Injects lines directly.
```rust
/// @openapi
/// paths:
///   /users:
///     get:
///       @insert CommonHeaders("X-Request-ID")
```

**Usage (Extend):** Merges content (useful for object composition).
```rust
/// @openapi
/// components:
///   schemas:
///     items:
///       @extend Timestamps
```

### 2. Generics (Schema Templates)

Define generic blueprints to avoid simple wrapper struct duplication.

**Definition:**
```rust
/// @openapi<T>
/// type: object
/// properties:
///   data:
///     $ref: $T
///   meta:
///     $ref: $PaginationMeta
struct Page<T>(T);
```

**Usage:**
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
///                 $ref: $Page<User>  <-- Generates Page_User schema
```

---

## Integration Guide (Axum & Swagger UI)

### Servicing with Axum & Swagger UI

Recommended pattern: Serve the spec dynamically and replace placeholders (like Auth URLs) at runtime.

**Implémentation:**

```rust
// src/routes/swagger.rs
use axum::{
  http::header,
  response::{Html, IntoResponse},
  routing::get,
  Router,
};
use crate::app::AppState; // Adjust import based on user project
use std::env;

const OPENAPI_SPEC: &str = include_str!("../../openapi.yaml");

// Define the placeholder used in Cargo.toml / openapi.yaml (if used as var)
// Or simply $$OIDC_URL in comments
const OIDC_PLACEHOLDER: &str = "$$OIDC_URL"; 
const DEFAULT_OIDC_URL: &str = "https://auth.example.com";

pub fn router() -> Router<AppState> {
  Router::new()
    .route("/openapi.yaml", get(serve_spec))
    .route("/swagger", get(serve_ui))
}

/// @openapi
/// paths:
///   /docs/openapi.yaml:
///     get:
///       tags: [System]
///       summary: Get OpenAPI Specification
///       description: Returns the dynamic OpenAPI 3.1.1 spec.
async fn serve_spec() -> impl IntoResponse {
  let oidc_url = env::var("OIDC_URL").unwrap_or_else(|_| DEFAULT_OIDC_URL.to_string());
  // Runtime replacement of the placeholder
  let dynamic_spec = OPENAPI_SPEC.replace(OIDC_PLACEHOLDER, &oidc_url);

  (
    [(header::CONTENT_TYPE, "application/yaml")],
    dynamic_spec,
  )
}

/// @openapi
/// paths:
///   /docs/swagger:
///     get:
///       summary: Swagger UI
async fn serve_ui() -> impl IntoResponse {
  let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <title>Swagger UI</title>
    <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5.11.0/swagger-ui.css" />
</head>
<body>
    <div id="swagger-ui"></div>
    <script src="https://unpkg.com/swagger-ui-dist@5.11.0/swagger-ui-bundle.js"></script>
    <script>
        window.onload = () => {
            window.ui = SwaggerUIBundle({
                url: '/docs/openapi.yaml',
                dom_id: '#swagger-ui',
                deepLinking: true,
                presets: [SwaggerUIBundle.presets.apis, SwaggerUIBundle.SwaggerUIStandalonePreset],
                layout: "BaseLayout",
            });
        };
    </script>
</body>
</html>"#;
  Html(html)
}
```

## License
MIT
