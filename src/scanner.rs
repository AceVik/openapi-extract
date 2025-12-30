use crate::error::{Error, Result};
use crate::visitor;
use std::collections::HashSet;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Replaces {{CARGO_PKG_VERSION}} with the environment variable or default.
pub fn substitute_variables(content: &str) -> String {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    content.replace("{{CARGO_PKG_VERSION}}", &version)
}

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

/// Scans directories for .rs files and other allowed types.
/// Returns a list of strings (OpenAPI snippets or full file contents)
/// with variable substitution AND smart reference substitution applied.
pub fn scan_directories(roots: &[PathBuf], includes: &[PathBuf]) -> Result<Vec<String>> {
    let mut files_found = false;

    // Intermediate collection
    let mut raw_snippets: Vec<String> = Vec::new();
    let mut defined_schemas: HashSet<String> = HashSet::new();

    // 1. Process directories
    for root in roots {
        for entry in WalkDir::new(root) {
            let entry = entry.map_err(|e| Error::Io(std::io::Error::other(e)))?;
            let path = entry.path();

            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    match ext {
                        "rs" => {
                            files_found = true;
                            let extracted = visitor::extract_from_file(path.to_path_buf())?;

                            for snippet in extracted.snippets {
                                raw_snippets.push(substitute_variables(&snippet));
                            }
                            defined_schemas.extend(extracted.schemas);
                        }
                        "json" | "yaml" | "yml" => {
                            files_found = true;
                            let content = std::fs::read_to_string(path)?;
                            raw_snippets.push(substitute_variables(&content));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // 2. Process explicit includes
    for path in includes {
        if path.exists() {
            files_found = true;
            if path.extension().is_some_and(|ext| ext == "rs") {
                let extracted = visitor::extract_from_file(path.to_path_buf())?;
                for snippet in extracted.snippets {
                    raw_snippets.push(substitute_variables(&snippet));
                }
                defined_schemas.extend(extracted.schemas);
            } else {
                let content = std::fs::read_to_string(path)?;
                raw_snippets.push(substitute_variables(&content));
            }
        }
    }

    if !files_found && !roots.is_empty() && !includes.is_empty() {
        return Err(Error::NoFilesFound);
    }

    log::info!(
        "Found {} defined schemas from Rust structs/enums: {:?}",
        defined_schemas.len(),
        defined_schemas
    );

    // 3. Apply Smart Reference Substitution
    let final_snippets: Vec<String> = raw_snippets
        .into_iter()
        .map(|s| substitute_smart_references(&s, &defined_schemas))
        .collect();

    Ok(final_snippets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smart_ref_replacement() {
        let mut schemas = HashSet::new();
        schemas.insert("User".to_string());
        schemas.insert("CreateUserDto".to_string());

        let input = "schema: $ref: $User";
        let output = substitute_smart_references(input, &schemas);
        assert_eq!(output, "schema: $ref: \"#/components/schemas/User\"");

        let input2 = "nested: { $ref: $CreateUserDto }";
        let output2 = substitute_smart_references(input2, &schemas);
        assert_eq!(
            output2,
            "nested: { $ref: \"#/components/schemas/CreateUserDto\" }"
        );

        // Non-matching
        let input3 = "price: $100";
        let output3 = substitute_smart_references(input3, &schemas);
        assert_eq!(output3, "price: $100"); // No replacement
    }
}
