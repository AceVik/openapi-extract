use crate::index::Registry;
use regex::Regex;
use std::sync::OnceLock;

static INSERT_RE: OnceLock<Regex> = OnceLock::new();
static EXTEND_RE: OnceLock<Regex> = OnceLock::new();

/// Pre-processes a snippet by expanding @insert and @extend directives.
pub fn preprocess(content: &str, registry: &Registry) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut new_lines = Vec::new();

    // Initialize Regexes once
    // Support optional args: @insert Name OR @insert Name(args)
    // Regex: @insert\s+([Ident])(?:\((.*)\))?
    let insert_re =
        INSERT_RE.get_or_init(|| Regex::new(r"@insert\s+([a-zA-Z0-9_]+)(?:\((.*)\))?").unwrap());
    let extend_re =
        EXTEND_RE.get_or_init(|| Regex::new(r"@extend\s+([a-zA-Z0-9_]+)(?:\((.*)\))?").unwrap());

    // Helper to parse args from regex capture
    fn parse_args_from_caps(args_str: Option<regex::Match>) -> Vec<String> {
        match args_str {
            Some(m) => {
                let s = m.as_str();
                if s.trim().is_empty() {
                    Vec::new()
                } else {
                    s.split(',')
                        .map(|x| x.trim().trim_matches('"').to_string())
                        .collect()
                }
            }
            None => Vec::new(),
        }
    }

    // Phase A: Textual Preparation
    // @insert -> text injection
    // @extend -> x-openapi-extend injection

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        if let Some(caps) = insert_re.captures(line) {
            // @insert logic (Textual)
            let name = caps.get(1).unwrap().as_str();
            let args = parse_args_from_caps(caps.get(2));

            if let Some(fragment) = registry.fragments.get(name) {
                let expanded = substitute_fragment_args(&fragment.body, &fragment.params, &args);
                let indent = line
                    .chars()
                    .take_while(|c| c.is_whitespace())
                    .collect::<String>();
                if !expanded.trim().is_empty() {
                    for frag_line in expanded.lines() {
                        new_lines.push(format!("{}{}", indent, frag_line));
                    }
                }
            } else {
                log::warn!("Fragment '{}' not found for @insert", name);
                new_lines.push(line.to_string());
            }
        } else if let Some(caps) = extend_re.captures(line) {
            // @extend logic (AST Marker)
            let name = caps.get(1).unwrap().as_str();
            let args_raw = caps.get(2).map(|m| m.as_str()).unwrap_or("");

            // We preserve indentation and inject a special key.
            // x-openapi-extend: "Name(arg1, arg2)"
            let indent = line
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect::<String>();
            // If args exist, format as Name(args), else Name
            let marker_val = if args_raw.is_empty() {
                name.to_string()
            } else {
                format!("{}({})", name, args_raw)
            };
            new_lines.push(format!("{}x-openapi-extend: \"{}\"", indent, marker_val));
        } else {
            new_lines.push(line.to_string());
        }
        i += 1;
    }

    let phase_a_output = new_lines.join("\n");

    // Phase B: Structural Merge
    // Try to parse as YAML Value. If fails, return textual output (fallback).
    match serde_yaml::from_str::<serde_yaml::Value>(&phase_a_output) {
        Ok(mut root) => {
            process_value(&mut root, registry);
            serde_yaml::to_string(&root).unwrap_or(phase_a_output)
        }
        Err(_) => {
            // Likely a partial snippet (list item or partial object).
            // Return text, but @extend markers are present.
            // If it's a snippet, @extend might not work fully structurally.
            // For now, we return phase_a_output.
            // (User Note: Snippet must be valid YAML for @extend to work structurally)
            phase_a_output
        }
    }
}

fn process_value(val: &mut serde_yaml::Value, registry: &Registry) {
    if let serde_yaml::Value::Mapping(map) = val {
        // Check for x-openapi-extend
        let extend_key = serde_yaml::Value::String("x-openapi-extend".to_string());

        let mut fragment_to_merge = None;

        if let Some(extend_val) = map.remove(&extend_key) {
            if let Some(extend_str) = extend_val.as_str() {
                fragment_to_merge = Some(extend_str.to_string());
            }
        }

        // Recurse children FIRST? or merge first?
        // Merge first so we can process children of merged result?
        // Or Process children then merge?
        // Usually merge first adds new keys, which might need processing.
        // But fragments are already "raw".
        // Let's merge first.

        if let Some(extend_str) = fragment_to_merge {
            // Parse "Name(args)"
            // reuse parse logic? We need simple parse here.
            let (name, args) = parse_extend_str(&extend_str);

            if let Some(fragment) = registry.fragments.get(&name) {
                let expanded = substitute_fragment_args(&fragment.body, &fragment.params, &args);
                if let Ok(frag_val) = serde_yaml::from_str::<serde_yaml::Value>(&expanded) {
                    merge_values(val, frag_val);
                } else {
                    log::warn!("Fragment '{}' body is not valid YAML", name);
                }
            } else {
                log::warn!("Fragment '{}' not found for @extend", name);
            }
        }

        // Recurse (re-borrow map after modification)
        // Check new keys too.
        if let serde_yaml::Value::Mapping(map) = val {
            for (_, v) in map {
                process_value(v, registry);
            }
        }
    } else if let serde_yaml::Value::Sequence(seq) = val {
        for v in seq {
            process_value(v, registry);
        }
    }
}

fn merge_values(target: &mut serde_yaml::Value, source: serde_yaml::Value) {
    match (target, source) {
        (serde_yaml::Value::Mapping(t_map), serde_yaml::Value::Mapping(s_map)) => {
            for (k, v) in s_map {
                if let Some(existing) = t_map.get_mut(&k) {
                    merge_values(existing, v);
                } else {
                    t_map.insert(k, v);
                }
            }
        }
        (t, s) => {
            *t = s;
        }
    }
}

fn parse_extend_str(s: &str) -> (String, Vec<String>) {
    if let Some(idx) = s.find('(') {
        let name = s[..idx].trim().to_string();
        let args_str = s[idx + 1..].trim_end_matches(')');
        let args = if args_str.trim().is_empty() {
            Vec::new()
        } else {
            args_str
                .split(',')
                .map(|x| x.trim().trim_matches('"').to_string())
                .collect()
        };
        (name, args)
    } else {
        (s.trim().to_string(), Vec::new())
    }
}

// Helper to substitute named args {{param}} in fragment
fn substitute_fragment_args(fragment: &str, params: &[String], args: &[String]) -> String {
    let mut result = fragment.to_string();
    for (i, param) in params.iter().enumerate() {
        if let Some(arg) = args.get(i) {
            let placeholder = format!("{{{{{}}}}}", param); // {{param}}
            result = result.replace(&placeholder, arg);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_with_indentation() {
        let mut registry = Registry::new();
        registry.insert_fragment(
            "Headers".to_string(),
            vec![],
            "header: x-val\nother: y-val".to_string(),
        );

        let input = "  @insert Headers(\"\")";
        let output = preprocess(input, &registry);

        // AST transformation normalizes indentation to root level
        let expected = "header: x-val\nother: y-val\n";
        assert_eq!(output, expected);
    }

    #[test]
    fn test_fragment_with_args() {
        let mut registry = Registry::new();
        registry.insert_fragment(
            "Field".to_string(),
            vec!["name".to_string()],
            "name: {{name}}".to_string(),
        );

        let input = "@insert Field(\"my-name\")";
        let output = preprocess(input, &registry);
        assert_eq!(output, "name: my-name\n");
    }

    #[test]
    fn test_missing_fragment() {
        let registry = Registry::new();
        let input = "@insert Missing(\"\")";
        let output = preprocess(input, &registry);
        // Fallback to text (phase A) because parsing might fail or pass
        // "@insert Missing" is likely treated as scalar string or invalid YAML?
        // "@insert Missing..." is just text.
        // If it parses as string, it returns string "...\n"
        // If it fails to parse (because of @?), it returns raw text.
        // "@" is reserved in YAML? At start of scalar?
        // Let's assert what we get.
        // In fallback path: same as input.
        assert_eq!(output, "@insert Missing(\"\")");
    }
}
