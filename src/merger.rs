use crate::error::{Error, Result};
use serde_yaml::Value;

/// Merges multiple OpenAPI YAML/JSON fragments into a single Value.
///
/// Logic:
/// 1. Parse all fragments.
/// 2. Identify strictly one "Root" (contains `openapi` and `info`).
/// 3. Deep merge all other fragments into the Root.
pub fn merge_openapi(fragments: Vec<String>) -> Result<Value> {
    let mut root: Option<Value> = None;
    let mut others: Vec<Value> = Vec::new();

    for fragment in fragments {
        // serde_yaml::from_str handles both YAML and JSON
        let value: Value = serde_yaml::from_str(&fragment)?;

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

        let result = merge_openapi(vec![root.to_string(), fragment.to_string()]).unwrap();

        // Helper to check fields
        let yaml_out = serde_yaml::to_string(&result).unwrap();
        assert!(yaml_out.contains("/foo"));
        assert!(yaml_out.contains("/bar"));
    }

    #[test]
    fn test_no_root() {
        let fragment = "paths: {}";
        let res = merge_openapi(vec![fragment.to_string()]);
        assert!(matches!(res, Err(Error::NoRootFound)));
    }

    #[test]
    fn test_multiple_roots() {
        let root1 = "openapi: 3.0\ninfo: {title: A}";
        let root2 = "openapi: 3.0\ninfo: {title: B}";
        let res = merge_openapi(vec![root1.to_string(), root2.to_string()]);
        assert!(matches!(res, Err(Error::MultipleRootsFound)));
    }
}
