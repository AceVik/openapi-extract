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
    let insert_re =
        INSERT_RE.get_or_init(|| Regex::new(r"@insert\s+([a-zA-Z0-9_]+)\((.*)\)").unwrap());
    let extend_re =
        EXTEND_RE.get_or_init(|| Regex::new(r"@extend\s+([a-zA-Z0-9_]+)\((.*)\)").unwrap());

    // Helper to parse args: "val1", "val2" -> Vec<String>
    fn parse_args(args_str: &str) -> Vec<String> {
        if args_str.trim().is_empty() {
            return Vec::new();
        }
        args_str
            .split(',')
            .map(|s| s.trim().trim_matches('"').to_string())
            .collect()
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

    for line in lines {
        if let Some(caps) = insert_re.captures(line) {
            let name = caps.get(1).unwrap().as_str();
            let args_str = caps.get(2).unwrap().as_str();
            let args = parse_args(args_str);

            if let Some(fragment) = registry.fragments.get(name) {
                let expanded = substitute_fragment_args(&fragment.body, &fragment.params, &args);
                // Maintain indentation
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
            let name = caps.get(1).unwrap().as_str();
            let args_str = caps.get(2).unwrap().as_str();
            let args = parse_args(args_str);

            if let Some(fragment) = registry.fragments.get(name) {
                let expanded_text =
                    substitute_fragment_args(&fragment.body, &fragment.params, &args);
                let indent = line
                    .chars()
                    .take_while(|c| c.is_whitespace())
                    .collect::<String>();

                // Smart Merge Logic (Regex Fallback)
                // If fragment has keys that look like they belong to a parent structure,
                // we just inject them.
                // But if we encounter duplicate keys...

                // For now, implementing simple injection but guarding against empty lines.
                if !expanded_text.trim().is_empty() {
                    for frag_line in expanded_text.lines() {
                        new_lines.push(format!("{}{}", indent, frag_line));
                    }
                }
            } else {
                log::warn!("Fragment '{}' not found for @extend", name);
                new_lines.push(line.to_string());
            }
        } else {
            new_lines.push(line.to_string());
        }
    }

    new_lines.join("\n")
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

        // Should maintain 2-space indentation
        assert_eq!(output, "  header: x-val\n  other: y-val");
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
        assert_eq!(output, "name: my-name");
    }

    #[test]
    fn test_missing_fragment() {
        let registry = Registry::new();
        let input = "@insert Missing(\"\")";
        let output = preprocess(input, &registry);
        // Should keep original line
        assert_eq!(output, "@insert Missing(\"\")");
    }
}
