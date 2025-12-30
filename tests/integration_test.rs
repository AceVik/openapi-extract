use oas_forge::scanner::scan_directories;
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
///   /macro-test:
///     parameters:
///       @insert QueryParam
///     get:
///       @extend MergeBase
///       responses:
///         '200':
///            description: OK override
///            content:
///              application/json:
///                schema:
///                  $ref: $Wrapper<User>
fn main() {{}}
    "#
    )
    .unwrap();

    // 4. Register MergeBase Fragment (Already done) / Register Param?
    // User wants @insert Param to be a ref NO MATTER WHAT if not in registry?
    // "QueryParam" is NOT in registry.
    // So it should become - $ref: "#/components/parameters/QueryParam"

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
    let merged = results
        .iter()
        .map(|s| s.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");

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

    // 6. Regression Check: Null Parameters
    // Ensure we don't generate "parameters:    // 6. Regression Check: Null Parameters
    assert!(!merged.contains("[null]"));
    assert!(!merged.contains("null"));

    // 7. DX Macros
    // @insert QueryParam -> - $ref: "#/components/parameters/QueryParam"
    // Note: serde_yaml might reformat quotes, so we check path presence.
    assert!(merged.contains("#/components/parameters/QueryParam"));

    // Generics Flattening: $Wrapper<User> -> #/components/schemas/Wrapper_User
    // Note: The original test used $Wrapper<User> and expected Monomorphizer to handle it.
    // Our NEW macro flattens it to #/components/schemas/Wrapper_User BEFORE Monomorphizer?
    // Wait. If Flattening happens BEFORE Monomorphizer, then Monomorphizer sees explicit Ref.
    // It DOES NOT see $Wrapper<User> syntax.
    // DOES Monomorphizer still generate Wrapper_User schema?
    // "Ensure the $ logic... applies".
    // If I flatten it to explicit Ref string, Monomorphizer (Pass 3) will NOT find `$Wrapper<User>`.
    // SO Wrapped_User concrete schema will NOT be generated!
    // THIS IS A BREAKING CHANGE if used on a new type.
    // UNLESS the user implies that `Generic` macro is only for usages of ALREADY generated types?
    // Or I need to fix `scanner.rs` logic.
    // Let's check `scanner.rs` `generic_re`.
    // It replaces "$Name<Arg>" with "#/components/schemas/Name_Arg".
    // Pass 3 (Monomorphizer) scans for `$Name<`.
    // Output of Pass 2a (Macro) has `#/components/schemas/Name_Arg`.
    // Pass 3 sees NO generics.
    // Result: `Wrapper_User` definition is MISSING in `registry.concrete_schemas`.
    // Result: Output has ref, but no definition!
    // TEST WILL FAIL on "Wrapper_User:" check (assertion 3).

    // FIX: I must ensure Monomorphizer is triggered.
    // Option A: `preprocess_macros` ALSO registers the intent?
    // Option B: `preprocess_macros` produces token that Monomorphizer recognizes?
    // Option C: `preprocess_macros` replaces `<` with `_` but keeps `$` prefix?
    //   Example: `$Wrapper_User`.
    //   Then Monomorphizer needs to be smart enough to parse `Wrapper_User` back to `Wrapper` and `User`. (Hard/Ambiguous).
    // Option D: I DON'T replace `<>` in `preprocess_macros` if it breaks Monomorphizer?
    // But user asked for it.
    // Maybe checking if it works is part of the test.
    // I will comment out the Generics part in `scanner.rs` or make it preserve the tag for Monomorphizer?
    // Wait, Monomorphizer scans `preprocessed_snippets`.
    // If I change it, Monomorphizer must handle the new format.

    // I will modify `scanner.rs` Generic Regex to:
    // NOT replace fully?
    // Or maybe I skip Generic Macro implementation for now to pass tests and fix later?
    // No, user requested it.

    // Correction:
    // If I assume `Monomorphizer` logic is moved to `macros`?
    // No.
    // I will assume `Wrapper_User` generation is handled SOMEHOW.
    // Actually, `Monomorphizer` logic looks for `$` char.
    // If I change `$Wrapper<User>` to `$Wrapper_User`?
    // `generics.rs` splits args.
    // It can't split `Wrapper_User`.

    // I will apply the Macro ONLY if it doesn't look like a definition?
    // No.

    // I will modify the test to manually define `Wrapper_User` via `@openapi` to simulate that it "exists"?
    // No, that defeats the purpose.

    // I will implement "Generics Flattening" by NOT destroying the `$` sigil completely,
    // AND updating `generics.rs` to handle `_` syntax? No.

    // REALIZATION:
    // The user's request for "Generics Flattening" as a "Macro" basically REPLACES the Monomorphizer's *detection* logic with a regex,
    // BUT fails to replace the *instantiation* logic.
    // Unless I move instantiation logic to `macros`?
    // Yes! `preprocess_macros` has `registry`.
    // I can replicate `monomorphizer.process` logic inside the regex replacement block!
    // "Logic: Replace < with _... Ensure $ logic applies".
    // If I can instantiate `Wrapper_User` right there in `preprocess_macros`, I don't need Pass 3 Monomorphizer for those matches!
    // I need `Monomorphizer` struct?
    // I can instantiate a temporary `Monomorphizer` inside `preprocess_macros`?
    // Yes.

    // PLAN UPDATE:
    // Update `preprocess_macros` to take `&mut Registry`.
    // Inside the Generic Regex matching:
    // 1. Extract Name, Args.
    // 2. Call `monomorphizer` logic (I need to expose `Monomorphizer::monomorphize` or equivalent).
    // 3. Return the Ref String `#/components/schemas/Name_Args`.

    // This connects everything!
    // I need to make `Monomorphizer::monomorphize` public? Or dup logic.
    // Making it public is better.
    // But `Monomorphizer` struct owns `&mut Registry`.
    // I can instantiate a temporary `Monomorphizer` inside `preprocess_macros`?
    // Yes.

    assert!(merged.contains("Wrapper_User:"));
}
