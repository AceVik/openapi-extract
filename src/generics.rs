use crate::index::Registry;
use std::collections::HashSet;

pub struct Monomorphizer<'a> {
    registry: &'a mut Registry,
    _processed_generics: HashSet<String>,
}

impl<'a> Monomorphizer<'a> {
    pub fn new(registry: &'a mut Registry) -> Self {
        Self {
            registry,
            _processed_generics: HashSet::new(),
        }
    }

    /// Scans text for generic patterns like $Page<User> and generates concrete schemas.
    /// Returns the text with $Page<User> replaced by $Page_User (which will be resolved to ref later).
    pub fn process(&mut self, content: &str) -> String {
        self.resolve_generics_in_text(content)
    }

    fn resolve_generics_in_text(&mut self, text: &str) -> String {
        let mut result = String::new();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1].is_alphabetic() {
                // Potential generic start
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let name: String = chars[start + 1..i].iter().collect();

                if i < chars.len() && chars[i] == '<' {
                    // It is a generic! $Name<
                    i += 1; // skip <
                    let arg_start = i;
                    let mut depth = 1;
                    while i < chars.len() && depth > 0 {
                        if chars[i] == '<' {
                            depth += 1;
                        } else if chars[i] == '>' {
                            depth -= 1;
                        }
                        i += 1;
                    }
                    // i is now after the closing >
                    // args_str exclude closing >
                    let args_str: String = chars[arg_start..i - 1].iter().collect();

                    // Create Concrete Schema
                    let concrete_name = self.monomorphize(&name, &args_str);

                    // Replace in text: $Page_User
                    result.push('$');
                    result.push_str(&concrete_name);
                } else {
                    // Just a regular $Name, push what we scanned
                    result.push_str(&text[start..i]);
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    /// Creates a concrete schema from a blueprint and args.
    /// e.g. Name="Page", Args="User" -> "Page_User"
    pub fn monomorphize(&mut self, name: &str, args_str: &str) -> String {
        // 1. Recursive resolve args (handle nested $Result<Page<User>>)
        let args = self.split_args(args_str);

        // 2. Normalize Args (e.g. resolve inner generics first)
        let resolved_args: Vec<String> = args
            .into_iter()
            .map(|arg| {
                if arg.contains('<') {
                    let processed = self.resolve_generics_in_text(&arg);
                    processed.trim_start_matches('$').to_string()
                } else {
                    arg.trim_start_matches('$').to_string()
                }
            })
            .collect();

        // 3. Generate Concrete Name
        let suffix = if resolved_args.is_empty() {
            "Generic".to_string()
        } else {
            resolved_args.join("_")
        };
        let concrete_name = format!("{}_{}", name, suffix);

        if self.registry.concrete_schemas.contains_key(&concrete_name) {
            return concrete_name;
        }

        // 4. Instantiate Blueprint
        if let Some(blueprint) = self.registry.blueprints.get(name).cloned() {
            let mut content = blueprint.body.clone();

            // Check arg count
            if resolved_args.len() != blueprint.params.len() {
                log::error!(
                    "Blueprint {} expects {} args, got {}. Using raw args.",
                    name,
                    blueprint.params.len(),
                    resolved_args.len()
                );
            }

            // Named Substitution: Replace $Param with $Arg
            for (idx, param) in blueprint.params.iter().enumerate() {
                if let Some(arg) = resolved_args.get(idx) {
                    // Pattern to replace: "$T" -> "$Arg"
                    // We replace literal "$" + param name
                    let target = format!("${}", param);
                    let replacement = format!("${}", arg);
                    content = content.replace(&target, &replacement);
                }
            }

            self.registry
                .concrete_schemas
                .insert(concrete_name.clone(), content);
        } else {
            log::warn!("Blueprint {} not found", name);
        }

        concrete_name
    }

    fn split_args(&self, args_str: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut start = 0;
        let mut depth = 0;
        let chars = args_str.char_indices().peekable();

        if args_str.trim().is_empty() {
            return Vec::new();
        }
        for (i, c) in chars {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                ',' if depth == 0 => {
                    args.push(args_str[start..i].trim().to_string());
                    // we need to skip the comma which is at i
                    start = i + 1;
                }
                _ => {}
            }
        }
        if start < args_str.len() {
            args.push(args_str[start..].trim().to_string());
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_monomorphize_named() {
        let mut registry = Registry::new();
        registry.insert_blueprint(
            "Page".to_string(),
            vec!["T".to_string()],
            "data: $ref: $T".to_string(),
        );

        let mut mono = Monomorphizer::new(&mut registry);
        let result = mono.process("scheme: $ref: $Page<User>");

        // Should generate Page_User
        assert_eq!(result, "scheme: $ref: $Page_User");

        // Verify concrete schema content
        let concrete = registry.concrete_schemas.get("Page_User").unwrap();
        assert_eq!(concrete, "data: $ref: $User");
    }

    #[test]
    fn test_nested_generics() {
        let mut registry = Registry::new();
        registry.insert_blueprint(
            "Wrapper".to_string(),
            vec!["T".to_string()],
            "wrap: $T".to_string(),
        );
        registry.insert_blueprint(
            "Inner".to_string(),
            vec!["U".to_string()],
            "in: $U".to_string(),
        );

        let mut mono = Monomorphizer::new(&mut registry);
        let result = mono.process("$Wrapper<$Inner<Item>>");

        assert_eq!(result, "$Wrapper_Inner_Item");

        // Verify intermediate
        assert!(registry.concrete_schemas.contains_key("Inner_Item"));
        let inner = registry.concrete_schemas.get("Inner_Item").unwrap();
        assert_eq!(inner, "in: $Item");

        // Verify outer
        assert!(registry.concrete_schemas.contains_key("Wrapper_Inner_Item"));
        let wrapper = registry.concrete_schemas.get("Wrapper_Inner_Item").unwrap();
        // Wrapper expects wrap: $T. T is Inner_Item. So wrap: $Inner_Item.
        assert_eq!(wrapper, "wrap: $Inner_Item");
    }
}
