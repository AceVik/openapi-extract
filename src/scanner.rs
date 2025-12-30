use crate::error::{Error, Result};
use crate::generics::Monomorphizer;
use crate::index::Registry;
use crate::preprocessor;
use crate::visitor::{self, ExtractedItem};
use regex::Regex;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::OnceLock;
use walkdir::WalkDir;

// DX Macros Preprocessor
// Implementation of auto-quoting and short-hands.
fn preprocess_macros(content: &str, registry: &mut Registry) -> String {
    let mut new_lines = Vec::new();

    // Regex for Generics: $Name<Arg>
    static GENERIC_RE: OnceLock<Regex> = OnceLock::new();
    let generic_re =
        GENERIC_RE.get_or_init(|| Regex::new(r"\$([a-zA-Z0-9_]+)<([a-zA-Z0-9_, ]+)>").unwrap());

    // Regex for Macros checks
    static MACRO_INSERT_RE: OnceLock<Regex> = OnceLock::new();
    let macro_insert_re = MACRO_INSERT_RE
        .get_or_init(|| Regex::new(r"^(\s*)(-)?\s*@insert\s+([a-zA-Z0-9_]+)$").unwrap());

    static MACRO_EXTEND_RE: OnceLock<Regex> = OnceLock::new();
    let macro_extend_re =
        MACRO_EXTEND_RE.get_or_init(|| Regex::new(r"^(\s*)@extend\s+(.+)$").unwrap());

    for line in content.lines() {
        // 1. Generics Flattening (Inline) + Instantiation
        let mut processed_line = line.to_string();

        while let Some(caps) = generic_re.captures(&processed_line.clone()) {
            let full_match = caps.get(0).unwrap().as_str();
            let name = caps.get(1).unwrap().as_str();
            let args_raw = caps.get(2).unwrap().as_str();

            // Instantiate via Monomorphizer
            let mut mono = Monomorphizer::new(registry);
            let concrete_name = mono.monomorphize(name, args_raw);

            // Replace with Smart Ref format ($Name)
            // This ensures it works as a reference key in YAML.
            let replacement = format!("${}", concrete_name);
            processed_line = processed_line.replace(full_match, &replacement);
        }

        // 2. Short-hand @insert
        if let Some(caps) = macro_insert_re.captures(&processed_line) {
            let indent = &caps[1];
            // caps[2] is dash, caps[3] is name
            let name = &caps[3];

            // Check registry (if not fragment, it's a ref)
            if !registry.fragments.contains_key(name) {
                // Force list item syntax: "- $ref: ..."
                // We always output indented list item.
                let final_indent = format!("{}- ", indent);
                new_lines.push(format!(
                    "{}$ref: \"#/components/parameters/{}\"",
                    final_indent, name
                ));
                continue;
            }
        }

        // 3. Auto-Quoting @extend
        if let Some(caps) = macro_extend_re.captures(&processed_line) {
            let indent = &caps[1];
            let content = &caps[2];

            // Output: x-openapi-extend: 'content'
            // Escape single quotes for YAML
            let escaped_content = content.replace('\'', "''");
            new_lines.push(format!("{}x-openapi-extend: '{}'", indent, escaped_content));
            continue;
        }

        new_lines.push(processed_line);
    }

    new_lines.join("\n")
}

/// Perform smart reference substitution ($Name -> #/components/schemas/Name)
pub fn substitute_smart_references(content: &str, schemas: &HashSet<String>) -> String {
    let mut result = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '$' {
            let mut j = i + 1;
            if j < chars.len() && (chars[j].is_alphabetic() || chars[j] == '_') {
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }

                let ident: String = chars[i + 1..j].iter().collect();

                if schemas.contains(&ident) {
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
                    continue;
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

fn finalize_substitution(content: &str) -> String {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    // Resolve literal escaping \$Ref -> $Ref
    let step1 = content.replace(r"\$", "$");
    step1.replace("{{CARGO_PKG_VERSION}}", &version)
}

pub fn scan_directories(roots: &[PathBuf], includes: &[PathBuf]) -> Result<Vec<String>> {
    let mut registry = Registry::new();
    let mut operation_snippets: Vec<String> = Vec::new();
    let mut files_found = false;

    let mut all_paths = Vec::new();
    for root in roots {
        for entry in WalkDir::new(root) {
            let entry = entry.map_err(|e| Error::Io(std::io::Error::other(e)))?;
            let path = entry.path().to_path_buf();
            if path.is_file() {
                all_paths.push(path);
            }
        }
    }
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

    // PASS 1: Indexing
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
                                operation_snippets.push(content);
                            }
                            ExtractedItem::Fragment {
                                name,
                                params,
                                content,
                            } => {
                                registry.insert_fragment(name, params, content);
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

    // PASS 2: Pre-Processing (Macros + Fragments)
    let mut preprocessed_snippets = Vec::new();
    for snippet in operation_snippets {
        // 2a. Expand Macros (and auto-instantiate generics)
        let macrod = preprocess_macros(&snippet, &mut registry);
        // 2b. Expand Fragments / Extend (Structural Merge)
        let expanded = preprocessor::preprocess(&macrod, &registry);
        preprocessed_snippets.push(expanded);
    }

    // PASS 3: Monomorphization
    // (Note: Autos-instantiated generics from macros already populated registry,
    // but explicit generic refs in blueprints might need processing still.
    // However, Monomorphizer.process primarily scans for Usage in snippets.
    // Since Macro replaced Usage with $Ref, Monomorphizer might not find them in snippets?
    // Wait.
    // Macro turns `$Page<User>` -> `$Page_User`.
    // Monomorphizer logic scans for `$Page<User>`.
    // So Pass 3 `monomorphizer.process` will see `$Page_User` (no brackets).
    // So it will NOT trigger?
    // Correct. That's why we called `mono.monomorphize` INSIDE `preprocess_macros`.
    // So we don't strictly need Pass 3 for those usage sites.
    // But we might need it for existing clean usages?
    // We keep it as a fallback or for non-macro usages (if any).

    let mut monomorphizer = Monomorphizer::new(&mut registry);
    let mut mono_snippets = Vec::new();

    for snippet in preprocessed_snippets {
        let mono = monomorphizer.process(&snippet);
        mono_snippets.push(mono);
    }

    // Inject Concrete Schemas generated by Monomorphizer
    let mut generated_snippets = Vec::new();
    for (name, content) in &registry.concrete_schemas {
        // Wrap concrete schemas (they are raw bodies from Blueprints)
        // Blueprints are usually full schemas?
        // If they are just properties, "type: object...", we need to wrap?
        // "Blueprint... usually contains the full schema definition".
        // BUT visitor auto-wrap might have wrapped it if it was a Schema?
        // Blueprints are extracted as `ExtractedItem::Blueprint`.
        // `visitor.rs` does NOT auto-wrap Blueprints currently (logic was only for Schema).
        // Let's assume Blueprints are typically defined with keys like `components:` or root level?
        // Or if the User defines:
        // /// @openapi<T>
        // /// type: object
        // Then it needs wrapping.
        // `Monomorphizer` uses variables.
        // If we inject it, we should probably wrap it similarly to Auto-Wrap?
        // Let's wrap it to be safe: components: schemas: {Name}: ...

        let wrapped = format!(
            "components:\n  schemas:\n    {}:\n{}",
            name,
            indent(content)
        );
        generated_snippets.push(wrapped);
    }
    mono_snippets.extend(generated_snippets);

    // PASS 4: Substitution
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
