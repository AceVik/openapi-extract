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

    // Helper to parse args: "val1", "val2" -> Vec<String>
    fn parse_args(args_str: Option<regex::Match>) -> Vec<String> {
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

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        if let Some(caps) = insert_re.captures(line) {
            let name = caps.get(1).unwrap().as_str();
            let args = parse_args(caps.get(2));

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
            i += 1;
        } else if let Some(caps) = extend_re.captures(line) {
            let name = caps.get(1).unwrap().as_str();
            let args = parse_args(caps.get(2));

            if let Some(fragment) = registry.fragments.get(name) {
                let expanded_text =
                    substitute_fragment_args(&fragment.body, &fragment.params, &args);
                let indent = line
                    .chars()
                    .take_while(|c| c.is_whitespace())
                    .collect::<String>();

                // Smart Merge:
                // 1. Identify top-level keys in Fragment
                // 2. Peek ahead in `lines` to see if they collide.
                // 3. If collide, SKIP the User's line (effectively replacing it with Fragment's logic which contains the key + children)
                //    Wait, logic check:
                //    Fragment: "responses:\n  200: ..."
                //    User: "@extend\nresponses:\n  400: ..."
                //    We inject Fragment. Result:
                //    "responses:\n  200: ..."
                //    Next user line is "responses:". We SKIP it.
                //    Next user line is "  400: ...". We Keep it.
                //    Result:
                //    "responses:\n  200: ...\n  400: ..." -> Valid Merge!

                // Parse keys from fragment (simple heuristic: lines ending in colon at indent 0 relative to fragment)
                // Using serde_yaml would be better but fragment might be partial.
                // Let's use robust string check: "key:" at start of line.

                let frag_keys: Vec<String> = expanded_text
                    .lines()
                    .filter_map(|l| {
                        let trimmed = l.trim();
                        if !l.starts_with(' ') && trimmed.ends_with(':') {
                            Some(trimmed.trim_end_matches(':').to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                // Inject fragment
                if !expanded_text.trim().is_empty() {
                    for frag_line in expanded_text.lines() {
                        new_lines.push(format!("{}{}", indent, frag_line));
                    }
                }

                // Look ahead for collisions
                // We only check the VERY NEXT line? Or scan until indent changes?
                // Usually the structure is immediate.
                // Checking next line is safe start.
                if i + 1 < lines.len() {
                    let next_line = lines[i + 1];
                    let next_trimmed = next_line.trim();
                    // If next line matches a key in fragment, skip it.
                    // But we must check indentation match?
                    // User's next line usually has same indentation as @extend?
                    // Yes.

                    if next_trimmed.ends_with(':') {
                        let key = next_trimmed.trim_end_matches(':');
                        if frag_keys.contains(&key.to_string()) {
                            // Collision! Skip this line so user's children merge into our injected key.
                            i += 1; // Skip the @extend line
                            i += 1; // Skip the colliding line
                            continue;
                        }
                    }
                }
            } else {
                log::warn!("Fragment '{}' not found for @extend", name);
                new_lines.push(line.to_string());
            }
            i += 1;
        } else {
            new_lines.push(line.to_string());
            i += 1;
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
