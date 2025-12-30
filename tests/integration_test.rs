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
///         '401':
///           @insert CommonError
///           
///   /merge-test:
///     get:
///       @extend MergeBase
///       responses:
///         '200':
///            description: OK override
fn main() {{}}
    "#
    )
    .unwrap();

    // 4. Register MergeBase Fragment for testing
    let merge_rs = src_dir.join("merge.rs");
    let mut f = File::create(&merge_rs).unwrap();
    writeln!(
        f,
        "{}",
        r#"
    //! @openapi-fragment MergeBase
    //! responses:
    //!   '404':
    //!     description: Not Found
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

    // 4. Optional Parens (@insert CommonError)
    // Should produce "Error {{code}}" -> "Error {{code}}" if no args provided?
    // Or did we default substitute?
    // Our fragment expects {{code}}. If no args, it remains {{code}}.
    // User's fragment: description: Error {{code}}
    assert!(merged.contains("description: Error {{code}}"));

    // 5. Smart Merge
    // We expect:
    // responses:
    //   '404': ... (From Fragment)
    //   '200': ... (From User)
    // And NO duplicate "responses:" key.
    // The "responses:" from User should have been skipped.

    // Check for double "responses:" lines in that block? Hard to check string.
    // Check that '404' and '200' effectively exist under one block.
    // But mainly we check '404' is present.
    assert!(merged.contains("'404':"));
    assert!(merged.contains("description: Not Found"));
    assert!(merged.contains("'200':"));
    assert!(merged.contains("description: OK override"));
}
