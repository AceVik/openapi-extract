use crate::error::{Error, Result};
use crate::scanner::Snippet;
use serde_yaml::Value;

/// Merges multiple OpenAPI YAML/JSON fragments into a single Value.
pub fn merge_openapi(snippets: Vec<Snippet>) -> Result<Value> {
    let mut root: Option<Value> = None;
    let mut others: Vec<Value> = Vec::new();

    for (_i, snippet) in snippets.iter().enumerate() {
        let value: Value = match serde_yaml::from_str(&snippet.content) {
            Ok(v) => v,
            Err(e) => {
                // Enhanced Error Reporting
                eprintln!("\n\x1b[31;1mERROR: YAML parsing failed\x1b[0m");
                eprintln!("  --> {}:{}", snippet.file_path.display(), snippet.line_number);
                eprintln!("  |");
                eprintln!("  = Reason: {}", e);
                eprintln!("  |");
                eprintln!("  = Snippet Context (first 5 lines):");
                for (idx, line) in snippet.content.lines().take(5).enumerate() {
                     eprintln!("    {:02} | {}", idx + snippet.line_number, line);
                }
                eprintln!();
                return Err(Error::Yaml(e));
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
        let s1 = Snippet { content: root1.to_string(), file_path: std::path::PathBuf::from("r1.yaml"), line_number: 1 };
        let s2 = Snippet { content: root2.to_string(), file_path: std::path::PathBuf::from("r2.yaml"), line_number: 1 };
        
        let res = merge_openapi(vec![s1, s2]);
        assert!(matches!(res, Err(Error::MultipleRootsFound)));
    }
}
