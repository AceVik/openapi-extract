use crate::error::{Error, Result};
use crate::scanner::Snippet;
use serde_yaml::Value;

/// Merges multiple OpenAPI YAML/JSON fragments into a single Value.
pub fn merge_openapi(snippets: Vec<Snippet>) -> Result<Value> {
    let mut root: Option<Value> = None;
    let mut others: Vec<Value> = Vec::new();

    for snippet in snippets {
        let value: Value = match serde_yaml::from_str(&snippet.content) {
            Ok(v) => v,
            Err(e) => {
                // Construct context string
                let context: String = snippet
                    .content
                    .lines()
                    .take(5)
                    .enumerate()
                    .map(|(idx, line)| format!("    {:02} | {}", idx + snippet.line_number, line))
                    .collect::<Vec<_>>()
                    .join("\n");

                return Err(Error::SourceMapped {
                    file: snippet.file_path.clone(),
                    line: snippet.line_number,
                    source: e,
                    context,
                });
            }
        };

        if is_root(&value) {
            if root.is_some() {
                return Err(Error::MultipleRootsFound);
            }
            root = Some(value);
        } else {
            others.push(value);
        }
    }

    let mut root = root.ok_or(Error::NoRootFound)?;

    for other in others {
        deep_merge(&mut root, other);
    }

    Ok(root)
}

fn is_root(value: &Value) -> bool {
    if let Value::Mapping(map) = value {
        map.contains_key("openapi") && map.contains_key("info")
    } else {
        false
    }
}

/// Recursive deep merge.
/// - Arrays: Appended.
/// - Maps: Merged recursively.
/// - Scalars: Overwritten by the source (right-hand side).
fn deep_merge(target: &mut Value, source: Value) {
    match (target, source) {
        (Value::Mapping(t_map), Value::Mapping(s_map)) => {
            for (key, s_val) in s_map {
                match t_map.get_mut(&key) {
                    Some(t_val) => deep_merge(t_val, s_val),
                    None => {
                        t_map.insert(key, s_val);
                    }
                }
            }
        }
        (Value::Sequence(t_seq), Value::Sequence(s_seq)) => {
            t_seq.extend(s_seq);
            // Deduplicate preserving order
            let mut seen = std::collections::HashSet::new();
            let mut unique = Vec::new();
            for item in t_seq.drain(..) {
                // We use the string representation for deduping to handle potential Hash/Eq oddities with YAML Values widely
                // But serde_yaml::Value does impl Hash/Eq.
                // However, let's trust serde_yaml's Hash implementation.
                if seen.insert(item.clone()) {
                    unique.push(item);
                }
            }
            *t_seq = unique;
        }
        (t, s) => {
            *t = s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_simple() {
        let root = r#"
        openapi: 3.0.0
        info:
          title: Test
          version: 1.0
        paths:
          /foo:
            get:
              description: root
        "#;

        let fragment = r#"
        paths:
          /bar:
            post:
              description: fragment
        "#;

        let root_snippet = Snippet {
            content: root.to_string(),
            file_path: std::path::PathBuf::from("root.yaml"),
            line_number: 1,
        };
        let frag_snippet = Snippet {
            content: fragment.to_string(),
            file_path: std::path::PathBuf::from("frag.yaml"),
            line_number: 1,
        };

        let result = merge_openapi(vec![root_snippet, frag_snippet]).unwrap();

        // Helper to check fields
        let yaml_out = serde_yaml::to_string(&result).unwrap();
        assert!(yaml_out.contains("/foo"));
        assert!(yaml_out.contains("/bar"));
    }

    #[test]
    fn test_no_root() {
        let fragment = "paths: {}";
        let snip = Snippet {
            content: fragment.to_string(),
            file_path: std::path::PathBuf::from("frag.yaml"),
            line_number: 1,
        };
        let res = merge_openapi(vec![snip]);
        assert!(matches!(res, Err(Error::NoRootFound)));
    }

    #[test]
    fn test_multiple_roots() {
        let root1 = "openapi: 3.0\ninfo: {title: A}";
        let root2 = "openapi: 3.0\ninfo: {title: B}";
        let s1 = Snippet {
            content: root1.to_string(),
            file_path: std::path::PathBuf::from("r1.yaml"),
            line_number: 1,
        };
        let s2 = Snippet {
            content: root2.to_string(),
            file_path: std::path::PathBuf::from("r2.yaml"),
            line_number: 1,
        };

        let res = merge_openapi(vec![s1, s2]);
        assert!(matches!(res, Err(Error::MultipleRootsFound)));
    }

    #[test]
    fn test_source_mapped_error() {
        let bad_yaml = "invalid: : yaml";
        let snippet = Snippet {
            content: bad_yaml.to_string(),
            file_path: std::path::PathBuf::from("bad.yaml"),
            line_number: 10,
        };
        let res = merge_openapi(vec![snippet]);
        match res {
            Err(Error::SourceMapped {
                file,
                line,
                context,
                ..
            }) => {
                assert_eq!(file.to_str().unwrap(), "bad.yaml");
                assert_eq!(line, 10);
                assert!(context.contains("invalid: : yaml"));
                assert!(context.contains("10 |")); // Line number in context
            }
            _ => panic!("Expected SourceMapped error"),
        }
    }
    #[test]
    fn test_merge_dedup() {
        // merge_openapi expects root detection (openapi/info).
        // But deep_merge is private.
        // We can test merge_openapi with full docs.

        let root_full = r#"
        openapi: 3.0.0
        info: {title: T, version: 1}
        tags: [A, B]
        "#;
        let frag_full = r#"
        tags: [B, C]
        "#;

        let r_snip = Snippet {
            content: root_full.to_string(),
            file_path: std::path::PathBuf::from("r"),
            line_number: 1,
        };
        let f_snip = Snippet {
            content: frag_full.to_string(),
            file_path: std::path::PathBuf::from("f"),
            line_number: 1,
        };

        let res = merge_openapi(vec![r_snip, f_snip]).unwrap();
        let yaml = serde_yaml::to_string(&res).unwrap();

        // Should contain A, B, C exactly once (though potentially reordered, B should not appear twice)
        // YAML output for list: - A\n- B\n- C
        // Count occurrences
        let count_b = yaml.matches("B").count();
        assert_eq!(count_b, 1, "Should deduplicate tag B");
        assert!(yaml.contains("A"));
        assert!(yaml.contains("C"));
    }
}
