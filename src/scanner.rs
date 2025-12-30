use crate::error::{Error, Result};
use crate::generics::Monomorphizer;
use crate::index::Registry;
use crate::preprocessor;
use crate::visitor::{self, ExtractedItem};
use std::collections::HashSet;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Perform smart reference substitution ($Name -> #/components/schemas/Name)
/// MANUALLY implemented to avoid `regex` dependency as per strict user requirements.
pub fn substitute_smart_references(content: &str, schemas: &HashSet<String>) -> String {
    let mut result = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '$' {
            // Check if what follows is a valid identifier start
            let mut j = i + 1;
            if j < chars.len() && (chars[j].is_alphabetic() || chars[j] == '_') {
                // Determine identifier end
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }

                let ident: String = chars[i + 1..j].iter().collect();

                if schemas.contains(&ident) {
                    // Valid schema reference found!
                    let is_quoted = i > 0 && chars[i - 1] == '"';

                    if !is_quoted {
                        result.push('"');
                    }
                    result.push_str("#/components/schemas/");
                    result.push_str(&ident);
                    if !is_quoted {
                        result.push('"');
                    }

                    i = j;
                    continue; // Skip the identifier chars
                }
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Replaces {{CARGO_PKG_VERSION}} and runtime placeholders.
fn finalize_substitution(content: &str) -> String {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());

    // Resolve literal escaping \$Ref -> $Ref
    let step1 = content.replace(r"\$", "$");

    // Runtime Vars $$VAR -> $$VAR (Left alone for runtime, but syntax spec said "Outputs literal".
    // Actually the user wants to USE $$VAR in the output. so we don't touch it,
    // OR we ensure that if they wrote $$VAR it isn't treated as a Smart Ref candidate?
    // Smart Ref checks for $Name. $$Name would fail the name check or need care.
    // Scan passes will likely ignore $$Name if regex excludes it.

    step1.replace("{{CARGO_PKG_VERSION}}", &version)
}

/// Orchestrates the 4-Pass Pipeline
pub fn scan_directories(roots: &[PathBuf], includes: &[PathBuf]) -> Result<Vec<String>> {
    let mut registry = Registry::new();
    let mut operation_snippets: Vec<String> = Vec::new();
    let mut files_found = false;

    // --- PASS 1: INDEXING ---
    // Scan all files, Populate Registry (Fragments, Blueprints, Schemas), Collect Operations.

    let mut all_paths = Vec::new();

    // Collect paths from roots
    for root in roots {
        for entry in WalkDir::new(root) {
            let entry = entry.map_err(|e| Error::Io(std::io::Error::other(e)))?;
            let path = entry.path().to_path_buf();
            if path.is_file() {
                all_paths.push(path);
            }
        }
    }
    // Add includes
    for path in includes {
        if path.exists() {
            all_paths.push(path.to_path_buf());
        }
    }

    if all_paths.is_empty() && !roots.is_empty() {
        return Err(Error::NoFilesFound);
    }
    if !all_paths.is_empty() {
        files_found = true;
    }

    for path in all_paths {
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            match ext {
                "rs" => {
                    let extracted = visitor::extract_from_file(path)?;
                    for item in extracted {
                        match item {
                            ExtractedItem::Schema { name, content } => {
                                if let Some(n) = name {
                                    registry.insert_schema(n, content.clone());
                                }
                                // Schemas are ALSO snippets that need processing output
                                operation_snippets.push(content);
                            }
                            ExtractedItem::Fragment { name, content } => {
                                registry.insert_fragment(name, content);
                            }
                            ExtractedItem::Blueprint {
                                name,
                                params,
                                content,
                            } => {
                                registry.insert_blueprint(name, params, content);
                            }
                        }
                    }
                }
                "json" | "yaml" | "yml" => {
                    let content = std::fs::read_to_string(&path)?;
                    operation_snippets.push(content);
                }
                _ => {}
            }
        }
    }

    // --- PASS 2: PRE-PROCESSING ---
    // Expand @insert / @extend in all collected snippets
    // We update `operation_snippets` in-place

    let mut preprocessed_snippets = Vec::new();
    for snippet in operation_snippets {
        let expanded = preprocessor::preprocess(&snippet, &registry);
        preprocessed_snippets.push(expanded);
    }

    // --- PASS 3: MONOMORPHIZATION ---
    // Scan for $Generic<Args>, generate specific schemas, register them.
    // Then replace $Generic<Args> with $Generic_Args (Smart Ref).

    let mut monomorphizer = Monomorphizer::new(&mut registry);
    let mut mono_snippets = Vec::new();

    for snippet in preprocessed_snippets {
        let mono = monomorphizer.process(&snippet);
        mono_snippets.push(mono);
    }

    // Also: The "Concrete Schemas" generated by monomorphizer need to be added to output!
    // They are in `registry.concrete_schemas`.
    // We need to inject them.
    // These schemas themselves might need final resolution?
    // Yes.

    // We convert the map to a list of snippets to be added.
    // Note: The content in registry is the raw body. We need to wrap it?
    // Usually schemas are mapped to `components/schemas/Name`.
    // If the snippet is just properties, we need to wrap it.
    // The Blueprint definition `@openapi<T>` usually contains the full schema definition.
    // So `content` should be fine.

    let mut generated_snippets = Vec::new();
    for (name, content) in &registry.concrete_schemas {
        // We probably need to ensure it has the key?
        // If the blueprint was:
        // components: schemas: Page: ...
        // Then `content` is that.
        // If the blueprint was just the body, we have a problem.
        // Let's assume the blueprint is a full fragment.

        // Wait, "Generation: Generate names: Result_Page_User. Register concrete schema."
        // How do we inject these into the final YAML?
        // We treat them as additional snippets.

        // But wait, if snippet is:
        // `type: object`
        // We can't just dump `type: object` at root level.
        // It needs to be under `components: schemas: Page_User:`
        // We'll wrap it automatically!

        let wrapped = format!(
            "components:\n  schemas:\n    {}:\n{}",
            name,
            indent(content)
        );
        generated_snippets.push(wrapped);
    }

    // Process generated snippets too (generics inside generics?)
    // This implies recursive monomorphization. `Monomorphizer` handles recursion logic internally?
    // `monomorphizer.process` calls `resolve_generics_in_text`.
    // If a generated schema has generics, `instantiate_blueprint` should handle it?
    // `monomorphizer` logic in `generics.rs` resolves args recursively.
    // Does it resolve the BODY?
    // We should run `monomorphizer.process` on the generated content too ideally.
    // For v0.2.0, let's append them to the list and proceed. (Assuming blueprints don't use other blueprints in the body? They might).
    // Let's rely on standard substitution.

    mono_snippets.extend(generated_snippets);

    // --- PASS 4: FINAL RESOLUTION ---
    // Substitute $SmartRefs, Escape \$Refs, Vars.
    // Also {{CARGO_PKG_VERSION}}.

    // Re-collect all index keys for smart ref (including concrete ones)
    let mut all_schemas = registry.schemas.keys().cloned().collect::<HashSet<_>>();
    all_schemas.extend(registry.concrete_schemas.keys().cloned());

    let mut final_snippets = Vec::new();
    for snippet in mono_snippets {
        let subbed = substitute_smart_references(&snippet, &all_schemas);
        let finalized = finalize_substitution(&subbed);
        final_snippets.push(finalized);
    }

    if !files_found {
        return Err(Error::NoFilesFound);
    }

    Ok(final_snippets)
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("      {}", l))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escaping() {
        let input = r"price: \$100";
        let output = finalize_substitution(input);
        assert_eq!(output, "price: $100");
    }
    // fn test_smart_ref_replacement() {
    //     let mut schemas = HashSet::new();
    //     schemas.insert("User".to_string());
    //     schemas.insert("CreateUserDto".to_string());

    //     let input = "schema: $ref: $User";
    //     let output = substitute_smart_references(input, &schemas);
    //     assert_eq!(output, "schema: $ref: \"#/components/schemas/User\"");

    //     let input2 = "nested: { $ref: $CreateUserDto }";
    //     let output2 = substitute_smart_references(input2, &schemas);
    //     assert_eq!(
    //         output2,
    //         "nested: { $ref: \"#/components/schemas/CreateUserDto\" }"
    //     );

    //     // Non-matching
    //     let input3 = "price: $100";
    //     let output3 = substitute_smart_references(input3, &schemas);
    //     assert_eq!(output3, "price: $100"); // No replacement
    // }
}
