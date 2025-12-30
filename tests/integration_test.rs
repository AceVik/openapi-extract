use openapi_extract::scanner::scan_directories;
use std::fs::File;
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_full_pipeline_v0_2_0() {
    let dir = tempdir().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir(&src_dir).unwrap();

    // 1. Define Fragment (//! comment)
    let lib_rs = src_dir.join("lib.rs");
    let mut f = File::create(&lib_rs).unwrap();
    writeln!(
        f,
        "{}",
        r#"
//! @openapi-fragment CommonError(code)
//! description: Error {{code}}
//! content:
//!   application/json:
//!     schema:
//!       $ref: $ErrorModel
    "#
    )
    .unwrap();

    // 2. Define Blueprint (/// @openapi<T>)
    let models_rs = src_dir.join("models.rs");
    let mut f = File::create(&models_rs).unwrap();
    writeln!(
        f,
        r#"
/// @openapi<T>
/// type: object
/// properties:
///   data: 
///     $ref: $T
struct Wrapper<T>(T);

/// @openapi
/// type: object
struct ErrorModel;

/// @openapi
/// type: object
struct User;
    "#
    )
    .unwrap();

    // 3. Define Output Schema using both
    let main_rs = src_dir.join("main.rs");
    let mut f = File::create(&main_rs).unwrap();
    writeln!(
        f,
        "{}",
        r#"
/// @openapi
/// paths:
///   /test:
///     get:
///       responses:
///         '200':
///           content:
///              application/json:
///                schema:
///                  $ref: $Wrapper<User>
///         '400':
///           @insert CommonError("Bad Request")
fn main() {{}}
    "#
    )
    .unwrap();

    // Execute
    let results = scan_directories(&[src_dir], &[]).expect("Scan failed");
    let merged = results.join("\n");

    // Assertions

    // 1. Fragment Insertion
    assert!(merged.contains("description: Error Bad Request"));

    // 2. Monomorphization (Wrapper_User)
    // The main code uses $Wrapper<User> -> $Wrapper_User
    // Passes 3 & 4 should resolve this.
    // The final output should contain a reference to Wrapper_User:
    assert!(merged.contains("$ref: \"#/components/schemas/Wrapper_User\""));

    // 3. Concrete Schema Generation
    // We expect the concrete schema to be injected into the output
    // components: schemas: Wrapper_User: ... data: $ref: "#/components/schemas/User"
    assert!(merged.contains("Wrapper_User:"));
    assert!(merged.contains("$ref: \"#/components/schemas/User\""));
}
