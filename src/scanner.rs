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

/// Represents a source-mapped snippet of OpenAPI definition.
#[derive(Debug, Clone)]
pub struct Snippet {
    pub content: String,
    pub file_path: PathBuf,
    pub line_number: usize,
}

// DX Macros Preprocessor
// Implementation of auto-quoting and short-hands.
fn preprocess_macros(snippet: &Snippet, registry: &mut Registry) -> Snippet {
    let content = &snippet.content;
    let mut new_lines = Vec::new();

    // Regex definition
    static GENERIC_RE: OnceLock<Regex> = OnceLock::new();
    let generic_re =
        GENERIC_RE.get_or_init(|| Regex::new(r"\$([a-zA-Z0-9_]+)<([a-zA-Z0-9_, ]+)>").unwrap());

    static MACRO_INSERT_RE: OnceLock<Regex> = OnceLock::new();
    let macro_insert_re = MACRO_INSERT_RE
        .get_or_init(|| Regex::new(r"^(\s*)(-)?\s*@insert\s+([a-zA-Z0-9_]+)$").unwrap());

    static MACRO_EXTEND_RE: OnceLock<Regex> = OnceLock::new();
    let macro_extend_re =
        MACRO_EXTEND_RE.get_or_init(|| Regex::new(r"^(\s*)@extend\s+(.+)$").unwrap());

    static MACRO_RETURN_RE: OnceLock<Regex> = OnceLock::new();
    let macro_return_re = MACRO_RETURN_RE.get_or_init(|| {
        Regex::new(r#"^(\s*)@return\s+(\d{3})\s*:\s*([^\s"]+)(?:\s+"(.*)")?$"#).unwrap()
    });

    static ARRAY_SHORT_RE: OnceLock<Regex> = OnceLock::new();
    let array_short_re =
        ARRAY_SHORT_RE.get_or_init(|| Regex::new(r"\$Vec<([a-zA-Z0-9_]+)>").unwrap());

    for line in content.lines() {
        let mut current_lines = vec![line.to_string()];

        // 0. Expand @return (Route Helper)
        if let Some(caps) = macro_return_re.captures(line) {
            let indent = &caps[1];
            let status = &caps[2];
            let schema_raw = &caps[3];
            let desc = caps.get(4).map(|m| m.as_str()).unwrap_or("Success");

            // If schema is $Vec<T>, it will be processed in the next step.
            // If schema is $User, we wrap it in $ref (unless it's already a ref?)
            // We assume the macro user provides a "Ref-like" string or a "$Vec" string.
            // We output "$ref: schema_raw" and let further passes resolve "$ref: $User".
            // However, if schema_raw is `$Vec<T>`, we want the result to be:
            // schema:
            //   type: array...
            // NOT `schema: $ref: { type: array }` -> Invalid.

            // Heuristic: If schema starts with `$Vec`, use it directly as the schema value.
            // Else use `$ref: schema_raw`.

            let schema_line = if schema_raw.starts_with("$Vec") {
                format!("{0}        {1}", indent, schema_raw) // Direct inject
            } else {
                format!("{0}        $ref: {1}", indent, schema_raw) // Ref inject
            };

            let expanded = format!(
                "{0}'{1}':\n{0}  description: \"{2}\"\n{0}  content:\n{0}    application/json:\n{0}      schema:\n{3}",
                indent, status, desc, schema_line
            );
            current_lines = expanded.lines().map(|s| s.to_string()).collect();
        }

        for sub_line in current_lines {
            let mut processed_line = sub_line.clone();

            // 1. Array Shorthand ($Vec<T>)
            // Replace ALL occurrences in the line
            while let Some(caps) = array_short_re.captures(&processed_line.clone()) {
                let full_match = caps.get(0).unwrap().as_str();
                let type_name = caps.get(1).unwrap().as_str();
                // Inline JSON syntax for array
                let replacement = format!(
                    "{{ type: array, items: {{ $ref: \"#/components/schemas/{}\" }} }}",
                    type_name
                );
                processed_line = processed_line.replace(full_match, &replacement);
            }

            // 2. Generics Flattening (Inline) + Instantiation
            // (Existing logic)
            while let Some(caps) = generic_re.captures(&processed_line.clone()) {
                let full_match = caps.get(0).unwrap().as_str();
                let name = caps.get(1).unwrap().as_str();
                let args_raw = caps.get(2).unwrap().as_str();

                // Instantiate via Monomorphizer
                let mut mono = Monomorphizer::new(registry);
                let concrete_name = mono.monomorphize(name, args_raw);

                // Replace with Smart Ref format ($Name)
                let replacement = format!("${}", concrete_name);
                processed_line = processed_line.replace(full_match, &replacement);
            }

            // 3. Short-hand @insert
            if let Some(caps) = macro_insert_re.captures(&processed_line) {
                let indent = &caps[1];
                let name = &caps[3];

                if !registry.fragments.contains_key(name) {
                    let final_indent = format!("{}- ", indent);
                    new_lines.push(format!(
                        "{}$ref: \"#/components/parameters/{}\"",
                        final_indent, name
                    ));
                    continue;
                }
            }

            // 4. Auto-Quoting @extend
            if let Some(caps) = macro_extend_re.captures(&processed_line) {
                let indent = &caps[1];
                let content = &caps[2];
                let escaped_content = content.replace('\'', "''");
                new_lines.push(format!("{}x-openapi-extend: '{}'", indent, escaped_content));
                continue;
            }

            new_lines.push(processed_line);
        }
    }

    Snippet {
        content: new_lines.join("\n"),
        file_path: snippet.file_path.clone(),
        line_number: snippet.line_number,
    }
}

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
    let step1 = content.replace(r"\$", "$");
    step1.replace("{{CARGO_PKG_VERSION}}", &version)
}

pub fn scan_directories(roots: &[PathBuf], includes: &[PathBuf]) -> Result<Vec<Snippet>> {
    let mut registry = Registry::new();
    let mut operation_snippets: Vec<Snippet> = Vec::new();
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

    if !all_paths.is_empty() {
        files_found = true;
    }

    // PASS 1: Indexing
    for path in all_paths {
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            match ext {
                "rs" => {
                    let extracted = visitor::extract_from_file(path.clone())?;
                    for item in extracted {
                        match item {
                            ExtractedItem::Schema {
                                name,
                                content,
                                line,
                            } => {
                                if let Some(n) = name {
                                    registry.insert_schema(n, content.clone());
                                }
                                operation_snippets.push(Snippet {
                                    content,
                                    file_path: path.clone(),
                                    line_number: line,
                                });
                            }
                            ExtractedItem::Fragment {
                                name,
                                params,
                                content,
                                ..
                            } => {
                                registry.insert_fragment(name, params, content);
                            }
                            ExtractedItem::Blueprint {
                                name,
                                params,
                                content,
                                ..
                            } => {
                                registry.insert_blueprint(name, params, content);
                            }
                        }
                    }
                }
                "json" | "yaml" | "yml" => {
                    let content = std::fs::read_to_string(&path)?;
                    operation_snippets.push(Snippet {
                        content,
                        file_path: path.clone(),
                        line_number: 1,
                    });
                }
                _ => {}
            }
        }
    }

    // PASS 2: Pre-Processing
    let mut preprocessed_snippets = Vec::new();
    for snippet in operation_snippets {
        // 2a. Expand Macros
        let macrod_snippet = preprocess_macros(&snippet, &mut registry);

        // 2b. Expand Fragments
        let expanded_content = preprocessor::preprocess(&macrod_snippet.content, &registry);

        preprocessed_snippets.push(Snippet {
            content: expanded_content,
            file_path: macrod_snippet.file_path,
            line_number: macrod_snippet.line_number,
        });
    }

    // PASS 3: Monomorphization
    let mut monomorphizer = Monomorphizer::new(&mut registry);
    let mut mono_snippets: Vec<Snippet> = Vec::new();

    for snippet in preprocessed_snippets {
        let mono_content = monomorphizer.process(&snippet.content);
        mono_snippets.push(Snippet {
            content: mono_content,
            file_path: snippet.file_path,
            line_number: snippet.line_number,
        });
    }

    // Inject Concrete Schemas
    let mut generated_snippets = Vec::new();
    for (name, content) in &registry.concrete_schemas {
        let wrapped = format!(
            "components:\n  schemas:\n    {}:\n{}",
            name,
            indent(content)
        );
        generated_snippets.push(Snippet {
            content: wrapped,
            file_path: PathBuf::from("<generated>"),
            line_number: 1,
        });
    }
    mono_snippets.extend(generated_snippets);

    // PASS 4: Substitution
    let mut all_schemas = registry.schemas.keys().cloned().collect::<HashSet<_>>();
    all_schemas.extend(registry.concrete_schemas.keys().cloned());

    let mut final_snippets = Vec::new();
    for snippet in mono_snippets {
        let subbed = substitute_smart_references(&snippet.content, &all_schemas);
        let finalized_content = finalize_substitution(&subbed);
        final_snippets.push(Snippet {
            content: finalized_content,
            file_path: snippet.file_path,
            line_number: snippet.line_number,
        });
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

    #[test]
    fn test_vec_macro() {
        let mut registry = Registry::new();
        let snippet = Snippet {
            content: "tags: $Vec<Tag>".to_string(),
            file_path: PathBuf::from("test.rs"),
            line_number: 1,
        };
        let processed = preprocess_macros(&snippet, &mut registry);
        assert!(processed.content.contains("type: array"));
        assert!(processed.content.contains("items:"));
        assert!(
            processed
                .content
                .contains("$ref: \"#/components/schemas/Tag\"")
        );
    }

    #[test]
    fn test_return_helper() {
        let mut registry = Registry::new();
        let snippet = Snippet {
            content: "@return 200: $User \"Success\"".to_string(),
            file_path: PathBuf::from("test.rs"),
            line_number: 1,
        };
        let processed = preprocess_macros(&snippet, &mut registry);
        assert!(processed.content.contains("'200':"));
        assert!(processed.content.contains("description: \"Success\""));
        assert!(processed.content.contains("schema:"));
        assert!(processed.content.contains("$ref: $User"));
    }

    #[test]
    fn test_return_helper_vec() {
        let mut registry = Registry::new();
        let snippet = Snippet {
            content: "@return 400: $Vec<Error>".to_string(),
            file_path: PathBuf::from("test.rs"),
            line_number: 1,
        };
        let processed = preprocess_macros(&snippet, &mut registry);
        assert!(processed.content.contains("'400':"));
        assert!(processed.content.contains("type: array"));
        assert!(
            processed
                .content
                .contains("$ref: \"#/components/schemas/Error\"")
        );
    }
}
