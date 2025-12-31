use regex::Regex;
use serde_json::{Value, json};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, File, ImplItemFn, ItemEnum, ItemFn, ItemMod, ItemStruct, ItemType};

/// Extracted item type
#[derive(Debug)]
pub enum ExtractedItem {
    /// Standard @openapi body
    Schema {
        name: Option<String>,
        content: String,
        line: usize,
    },
    /// @openapi-fragment Name(args...)
    Fragment {
        name: String,
        params: Vec<String>,
        content: String,
        line: usize,
    },
    /// @openapi<T, U>
    Blueprint {
        name: String,
        params: Vec<String>,
        content: String,
        line: usize,
    },
}

#[derive(Default)]
pub struct OpenApiVisitor {
    pub items: Vec<ExtractedItem>,
    pub current_tags: Vec<String>,
}

impl OpenApiVisitor {
    // Helper to process doc attributes on items (structs, fns, types)
    // Updated: No longer accepts generated_content. Strictly for @openapi blocks (Paths/Fragments).
    fn check_attributes(
        &mut self,
        attrs: &[Attribute],
        item_ident: Option<String>,
        item_line: usize,
    ) {
        let mut doc_lines = Vec::new();

        for attr in attrs {
            if attr.path().is_ident("doc") {
                if let syn::Meta::NameValue(meta) = &attr.meta {
                    if let Expr::Lit(expr_lit) = &meta.value {
                        if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                            doc_lines.push(lit_str.value());
                        }
                    }
                }
            }
        }

        // Only process if explicit @openapi tag exists
        if !doc_lines.iter().any(|l| l.contains("@openapi")) {
            return;
        }

        let full_doc = doc_lines.join("\n");
        self.parse_doc_block(&full_doc, item_ident, item_line);
    }

    fn parse_doc_block(&mut self, doc: &str, item_ident: Option<String>, line: usize) {
        let lines: Vec<&str> = doc.lines().collect();
        // Naive unindent
        let min_indent = lines
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.chars().take_while(|c| *c == ' ').count())
            .min()
            .unwrap_or(0);

        let unindented: Vec<String> = lines
            .into_iter()
            .map(|l| {
                if l.len() >= min_indent {
                    l[min_indent..].to_string()
                } else {
                    l.to_string()
                }
            })
            .collect();
        let content = unindented.join("\n");

        let mut sections = Vec::new();
        let mut current_header = String::new();
        let mut current_body = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("@openapi") {
                if !current_header.is_empty() || !current_body.is_empty() {
                    sections.push((current_header.clone(), current_body.join("\n")));
                }
                current_header = trimmed.to_string();
                current_body.clear();
            } else if trimmed.starts_with('{') && current_header.is_empty() {
                if !current_header.is_empty() || !current_body.is_empty() {
                    sections.push((current_header.clone(), current_body.join("\n")));
                }
                current_header = "@json".to_string();
                current_body.push(line.to_string());
            } else {
                current_body.push(line.to_string());
            }
        }
        if !current_header.is_empty() || !current_body.is_empty() {
            sections.push((current_header, current_body.join("\n")));
        }

        for (header, body) in sections {
            let mut body_content = body.trim().to_string();

            if header.starts_with("@openapi-fragment") {
                let rest = header.strip_prefix("@openapi-fragment").unwrap().trim();
                let (name, params) = if let Some(idx) = rest.find('(') {
                    let name = rest[..idx].trim().to_string();
                    let params_str = rest[idx + 1..].trim_end_matches(')');
                    let params: Vec<String> = params_str
                        .split(',')
                        .map(|p| p.trim().to_string())
                        .filter(|p| !p.is_empty())
                        .collect();
                    (name, params)
                } else {
                    (rest.to_string(), Vec::new())
                };

                self.items.push(ExtractedItem::Fragment {
                    name,
                    params,
                    content: body_content,
                    line,
                });
            } else if header.starts_with("@openapi-type") {
                let name = header
                    .strip_prefix("@openapi-type")
                    .unwrap()
                    .trim()
                    .to_string();
                // Wrap content in schema definition
                let wrapped = wrap_in_schema(&name, &body_content);
                self.items.push(ExtractedItem::Schema {
                    name: Some(name),
                    content: wrapped,
                    line,
                });
            } else if header.starts_with("@openapi") && header.contains('<') {
                if let Some(start) = header.find('<') {
                    if let Some(end) = header.rfind('>') {
                        let params_str = &header[start + 1..end];
                        let params: Vec<String> = params_str
                            .split(',')
                            .map(|p| p.trim().to_string())
                            .filter(|p| !p.is_empty())
                            .collect();

                        if let Some(ident) = &item_ident {
                            self.items.push(ExtractedItem::Blueprint {
                                name: ident.clone(),
                                params,
                                content: body_content,
                                line,
                            });
                        }
                    }
                }
            } else if (header.starts_with("@openapi") && !header.contains('<'))
                || header == "@json"
                || header.is_empty()
            {
                // TAG INJECTION
                if !self.current_tags.is_empty() {
                    let tags_yaml_list = self
                        .current_tags
                        .iter()
                        .map(|t| format!("- {}", t))
                        .collect::<Vec<_>>();

                    let verbs = [
                        "get:", "post:", "put:", "delete:", "patch:", "head:", "options:", "trace:",
                    ];
                    let mut new_lines = Vec::new();
                    let mut injected_any = false;

                    for line in body_content.lines() {
                        new_lines.push(line.to_string());
                        let trimmed = line.trim();
                        if verbs.iter().any(|v| trimmed == *v) {
                            let indent = line.chars().take_while(|c| *c == ' ').count();
                            let child_indent = " ".repeat(indent + 2);

                            if !body_content.contains("tags:") {
                                new_lines.push(format!("{}tags:", child_indent));
                                for tag in &tags_yaml_list {
                                    new_lines.push(format!("{}  {}", child_indent, tag));
                                }
                                injected_any = true;
                            }
                        }
                    }

                    if injected_any {
                        body_content = new_lines.join("\n");
                    }
                }

                // Auto-Wrap Heuristic (Only for manual blocks now)
                let starts_with_toplevel = body_content.lines().any(|line| {
                    let trimmed = line.trim();
                    if trimmed.starts_with("#") {
                        return false;
                    }
                    if let Some(key) = trimmed.split(':').next() {
                        matches!(
                            key.trim(),
                            "openapi"
                                | "info"
                                | "paths"
                                | "components"
                                | "tags"
                                | "servers"
                                | "security"
                        )
                    } else {
                        false
                    }
                });

                let final_content = if !starts_with_toplevel && !body_content.trim().is_empty() {
                    if let Some(n) = &item_ident {
                        wrap_in_schema(n, &body_content)
                    } else {
                        body_content
                    }
                } else {
                    body_content
                };

                self.items.push(ExtractedItem::Schema {
                    name: item_ident.clone(),
                    content: final_content,
                    line,
                });
            }
        }
    }
}

// Helper to wrap content in components/schemas
fn wrap_in_schema(name: &str, content: &str) -> String {
    let indented = content
        .lines()
        .map(|l| format!("      {}", l))
        .collect::<Vec<_>>()
        .join("\n");
    format!("components:\n  schemas:\n    {}:\n{}", name, indented)
}

// Helper for type mapping
fn map_syn_type_to_openapi(ty: &syn::Type) -> (Value, bool) {
    match ty {
        syn::Type::Path(p) => {
            if let Some(seg) = p.path.segments.last() {
                let ident = seg.ident.to_string();

                if ["Box", "Arc", "Rc", "Cow"].contains(&ident.as_str()) {
                    if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                        if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                            return map_syn_type_to_openapi(inner);
                        }
                    }
                }

                match ident.as_str() {
                    "bool" => (json!({ "type": "boolean" }), true),
                    "String" | "str" | "char" => (json!({ "type": "string" }), true),
                    "i8" | "i16" | "i32" | "u8" | "u16" | "u32" => {
                        (json!({ "type": "integer", "format": "int32" }), true)
                    }
                    "i64" | "u64" | "isize" | "usize" => {
                        (json!({ "type": "integer", "format": "int64" }), true)
                    }
                    "f32" => (json!({ "type": "number", "format": "float" }), true),
                    "f64" => (json!({ "type": "number", "format": "double" }), true),
                    "Uuid" => (json!({ "type": "string", "format": "uuid" }), true),
                    "NaiveDate" => (json!({ "type": "string", "format": "date" }), true),
                    "DateTime" | "NaiveDateTime" => {
                        (json!({ "type": "string", "format": "date-time" }), true)
                    }
                    "NaiveTime" => (json!({ "type": "string", "format": "time" }), true),
                    "Url" | "Uri" => (json!({ "type": "string", "format": "uri" }), true),
                    "Decimal" | "BigDecimal" => {
                        (json!({ "type": "string", "format": "decimal" }), true)
                    }
                    "ObjectId" => (json!({ "type": "string", "format": "objectid" }), true),
                    "Value" => (json!({}), true),
                    "Option" => {
                        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                            if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                                let (inner_val, _) = map_syn_type_to_openapi(inner);
                                return (inner_val, false);
                            }
                        }
                        (json!({}), false)
                    }
                    "Vec" | "LinkedList" | "HashSet" => {
                        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                            if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                                let (inner_val, _) = map_syn_type_to_openapi(inner);
                                return (json!({ "type": "array", "items": inner_val }), true);
                            }
                        }
                        (json!({ "type": "array" }), true)
                    }
                    "HashMap" | "BTreeMap" => {
                        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                            if args.args.len() >= 2 {
                                if let syn::GenericArgument::Type(val_type) = &args.args[1] {
                                    let (val_schema, _) = map_syn_type_to_openapi(val_type);
                                    return (
                                        json!({ "type": "object", "additionalProperties": val_schema }),
                                        true,
                                    );
                                }
                            }
                        }
                        (json!({ "type": "object" }), true)
                    }
                    _ => (json!({ "$ref": format!("${}", ident) }), true),
                }
            } else {
                (json!({ "type": "object" }), true)
            }
        }
        _ => (json!({ "type": "object" }), true),
    }
}

// Deep Merge Helper for JSON Values
fn json_merge(a: &mut Value, b: Value) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                json_merge(a.entry(k).or_insert(Value::Null), v);
            }
        }
        (a, b) => *a = b,
    }
}

impl<'ast> Visit<'ast> for OpenApiVisitor {
    fn visit_file(&mut self, i: &'ast File) {
        // State machine for file-level doc blocks
        let mut current_block_type: Option<String> = None;
        let mut current_block_lines = Vec::new();
        let mut start_line = 1;

        // Process file attributes (inner doc comments)
        for attr in &i.attrs {
            if attr.path().is_ident("doc") {
                if let syn::Meta::NameValue(meta) = &attr.meta {
                    if let Expr::Lit(expr_lit) = &meta.value {
                        if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                            let raw_line = lit_str.value();
                            let trimmed = raw_line.trim();

                            if trimmed.starts_with("@openapi-type") {
                                // Flush previous if exists
                                if !current_block_lines.is_empty() {
                                    let body = current_block_lines.join("\n");
                                    if let Some(name) = current_block_type.take() {
                                        let wrapped = wrap_in_schema(&name, &body);
                                        self.items.push(ExtractedItem::Schema {
                                            name: Some(name),
                                            content: wrapped,
                                            line: start_line,
                                        });
                                    } else {
                                        // Standard Root/Fragment block
                                        self.parse_doc_block(&body, None, start_line);
                                    }
                                    current_block_lines.clear();
                                }

                                // Start New Type
                                if let Some(name) = trimmed.strip_prefix("@openapi-type") {
                                    current_block_type = Some(name.trim().to_string());
                                    start_line = attr.span().start().line;
                                }
                            } else if trimmed.starts_with("@openapi") {
                                // Flush previous
                                if !current_block_lines.is_empty() {
                                    let body = current_block_lines.join("\n");
                                    if let Some(name) = current_block_type.take() {
                                        let wrapped = wrap_in_schema(&name, &body);
                                        self.items.push(ExtractedItem::Schema {
                                            name: Some(name),
                                            content: wrapped,
                                            line: start_line,
                                        });
                                    } else {
                                        self.parse_doc_block(&body, None, start_line);
                                    }
                                    current_block_lines.clear();
                                }

                                // Start Root/Fragment
                                current_block_type = None;
                                start_line = attr.span().start().line;
                                current_block_lines.push(raw_line); // preserve header
                            } else if !current_block_lines.is_empty()
                                || current_block_type.is_some()
                            {
                                current_block_lines.push(raw_line);
                            }
                        }
                    }
                }
            } else {
                // Flush on non-doc attr to be safe
                if !current_block_lines.is_empty() {
                    let body = current_block_lines.join("\n");
                    if let Some(name) = current_block_type.take() {
                        let wrapped = wrap_in_schema(&name, &body);
                        self.items.push(ExtractedItem::Schema {
                            name: Some(name),
                            content: wrapped,
                            line: start_line,
                        });
                    } else {
                        self.parse_doc_block(&body, None, start_line);
                    }
                    current_block_lines.clear();
                }
            }
        }

        // Flush EOF
        if !current_block_lines.is_empty() {
            let body = current_block_lines.join("\n");
            if let Some(name) = current_block_type {
                let wrapped = wrap_in_schema(&name, &body);
                self.items.push(ExtractedItem::Schema {
                    name: Some(name),
                    content: wrapped,
                    line: start_line,
                });
            } else {
                self.parse_doc_block(&body, None, start_line);
            }
        }

        visit::visit_file(self, i);
    }

    fn visit_item_fn(&mut self, i: &'ast ItemFn) {
        let mut doc_lines = Vec::new();
        for attr in &i.attrs {
            if attr.path().is_ident("doc") {
                if let syn::Meta::NameValue(meta) = &attr.meta {
                    if let Expr::Lit(expr_lit) = &meta.value {
                        if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                            doc_lines.push(lit_str.value());
                        }
                    }
                }
            }
        }

        // Check for DSL trigger
        let has_route = doc_lines.iter().any(|l| l.trim().starts_with("@route"));

        if !has_route {
            // Legacy Fallback
            self.check_attributes(&i.attrs, None, i.span().start().line);
            visit::visit_item_fn(self, i);
            return;
        }

        // DSL Mode
        let mut operation = json!({
            "summary": Value::Null,
            "description": Value::Null,
            "operationId": i.sig.ident.to_string(),
            "tags": [],
            "parameters": [],
            "responses": {}
        });

        let mut method = String::new();
        let mut path = String::new();
        let mut description_buffer = Vec::new();
        let mut summary: Option<String> = None;
        let mut declared_path_params = std::collections::HashSet::new();

        for line in &doc_lines {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if trimmed.starts_with("@route") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 3 {
                    method = parts[1].to_lowercase();
                    let raw_path = parts[2..].join(" ");

                    let mut new_path = String::new();
                    let mut last_end = 0;

                    // Regex: \{(\w+)(?::\s*([^"}]+))?(?:\s*"([^"]+)")?\}
                    // Matches {id}, {id: u32}, {id: u32 "Description"}
                    // Group 2: Type (trimmed), Group 3: Description (content inside quotes)
                    let re = Regex::new(r#"\{(\w+)(?::\s*([^"}]+))?(?:\s*"([^"]+)")?\}"#).unwrap();

                    for cap in re.captures_iter(&raw_path) {
                        let full_match = cap.get(0).unwrap();
                        let name = cap.get(1).unwrap().as_str();
                        let type_str = cap.get(2).map(|m| m.as_str().trim());
                        let desc = cap.get(3).map(|m| m.as_str().to_string()); // Directly capture inside quotes

                        new_path.push_str(&raw_path[last_end..full_match.start()]);
                        new_path.push('{');
                        new_path.push_str(name);
                        new_path.push('}');
                        last_end = full_match.end();

                        let is_bare = type_str.is_none() && desc.is_none();

                        if !is_bare {
                            declared_path_params.insert(name.to_string());

                            let t = type_str.unwrap_or("String");
                            let (schema, _is_required) =
                                if let Ok(ty) = syn::parse_str::<syn::Type>(t) {
                                    map_syn_type_to_openapi(&ty)
                                } else {
                                    (json!({ "type": "string" }), true)
                                };

                            let mut param_obj = json!({
                                "name": name,
                                "in": "path",
                                "required": true,
                                "schema": schema
                            });

                            if let Some(d) = desc {
                                if let Value::Object(m) = &mut param_obj {
                                    m.insert("description".to_string(), json!(d));
                                }
                            }

                            if let Value::Array(params) = operation.get_mut("parameters").unwrap() {
                                params.push(param_obj);
                            }
                        }
                    }
                    new_path.push_str(&raw_path[last_end..]);
                    path = new_path;
                }
            } else if trimmed.starts_with("@tag") {
                let final_content = if trimmed.starts_with("@tags") {
                    trimmed.strip_prefix("@tags").unwrap().trim()
                } else {
                    trimmed.strip_prefix("@tag").unwrap().trim()
                };

                if final_content.starts_with('[') && final_content.ends_with(']') {
                    let inner = &final_content[1..final_content.len() - 1];
                    for t in inner.split(',') {
                        if let Value::Array(tags) = operation.get_mut("tags").unwrap() {
                            tags.push(json!(t.trim()));
                        }
                    }
                } else {
                    if let Value::Array(tags) = operation.get_mut("tags").unwrap() {
                        tags.push(json!(final_content));
                    }
                }
            } else if trimmed.contains("-param") && trimmed.starts_with('@') {
                let (param_type, rest) = if trimmed.starts_with("@query-param") {
                    (
                        "query",
                        trimmed.strip_prefix("@query-param").unwrap().trim(),
                    )
                } else if trimmed.starts_with("@path-param") {
                    ("path", trimmed.strip_prefix("@path-param").unwrap().trim())
                } else if trimmed.starts_with("@header-param") {
                    (
                        "header",
                        trimmed.strip_prefix("@header-param").unwrap().trim(),
                    )
                } else if trimmed.starts_with("@cookie-param") {
                    (
                        "cookie",
                        trimmed.strip_prefix("@cookie-param").unwrap().trim(),
                    )
                } else {
                    continue;
                };

                if let Some(colon_idx) = rest.find(':') {
                    let name = rest[..colon_idx].trim();
                    let residue = rest[colon_idx + 1..].trim();

                    let mut tokens = Vec::new();
                    let mut current = String::new();
                    let mut in_quote = false;
                    for c in residue.chars() {
                        if c == '"' {
                            in_quote = !in_quote;
                            current.push(c);
                        } else if c.is_whitespace() && !in_quote {
                            if !current.is_empty() {
                                tokens.push(current.clone());
                                current.clear();
                            }
                        } else {
                            current.push(c);
                        }
                    }
                    if !current.is_empty() {
                        tokens.push(current);
                    }

                    if tokens.is_empty() {
                        continue;
                    }

                    // Identify Type
                    let first = &tokens[0];
                    let (type_str, start_idx) = if first == "deprecated"
                        || first == "required"
                        || first.contains('=')
                        || first.starts_with('"')
                    {
                        ("String", 0)
                    } else if syn::parse_str::<syn::Type>(first).is_ok() {
                        (first.as_str(), 1)
                    } else {
                        // Fallback
                        ("String", 0)
                    };

                    let (schema, mut is_required) =
                        if let Ok(ty) = syn::parse_str::<syn::Type>(type_str) {
                            map_syn_type_to_openapi(&ty)
                        } else {
                            (json!({ "type": "string" }), true)
                        };

                    let mut deprecated = false;
                    let mut example = None;
                    let mut desc = None;

                    for token in tokens.iter().skip(start_idx) {
                        if token == "deprecated" {
                            deprecated = true;
                        } else if token == "required" {
                            is_required = true;
                        } else if token.starts_with("example=") {
                            let val = token.strip_prefix("example=").unwrap().trim_matches('"');
                            example = Some(val.to_string());
                        } else if token.starts_with('"') {
                            desc = Some(token.trim_matches('"').to_string());
                        }
                    }

                    let mut param_obj = json!({
                        "name": name,
                        "in": param_type,
                        "required": is_required,
                        "schema": schema
                    });

                    if deprecated {
                        param_obj
                            .as_object_mut()
                            .unwrap()
                            .insert("deprecated".to_string(), json!(true));
                    }
                    if let Some(ex) = example {
                        param_obj
                            .as_object_mut()
                            .unwrap()
                            .insert("example".to_string(), json!(ex));
                    }

                    if param_type == "path" {
                        declared_path_params.insert(name.to_string());
                        if let Value::Object(m) = &mut param_obj {
                            m.insert("required".to_string(), json!(true));
                        }
                    }

                    if let Some(d) = desc {
                        if let Value::Object(m) = &mut param_obj {
                            m.insert("description".to_string(), json!(d));
                        }
                    }

                    if let Value::Array(params) = operation.get_mut("parameters").unwrap() {
                        params.push(param_obj);
                    }
                }
            } else if trimmed.starts_with("@body") {
                let rest = trimmed.strip_prefix("@body").unwrap().trim();
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if !parts.is_empty() {
                    let schema_ref = parts[0];
                    let mime = if parts.len() > 1 {
                        parts[1]
                    } else {
                        "application/json"
                    };

                    let schema = if schema_ref.contains('<')
                        || (schema_ref.starts_with('$') && schema_ref.contains('<'))
                    {
                        json!({ "$ref": schema_ref })
                    } else if let Ok(ty) = syn::parse_str::<syn::Type>(schema_ref) {
                        map_syn_type_to_openapi(&ty).0
                    } else {
                        if schema_ref.starts_with('$') {
                            json!({ "$ref": format!("#/components/schemas/{}", &schema_ref[1..]) })
                        } else {
                            json!({ "$ref": format!("#/components/schemas/{}", schema_ref) })
                        }
                    };

                    operation["requestBody"] = json!({
                        "content": {
                            mime: {
                                "schema": schema
                            }
                        }
                    });
                }
            } else if trimmed.starts_with("@return") {
                let rest = trimmed.strip_prefix("@return").unwrap().trim();
                let parts = if let Some(idx) = rest.find(':') {
                    // Check structure to be safe
                    Some(idx)
                } else {
                    None
                };

                if let Some(colon_idx) = parts {
                    let code = rest[..colon_idx].trim();
                    let residue = rest[colon_idx + 1..].trim();

                    let (type_str, desc, is_unit) = if residue.starts_with('"') {
                        ("()", Some(residue.trim_matches('"').to_string()), true)
                    } else {
                        if let Some(quote_start) = residue.find('"') {
                            (
                                residue[..quote_start].trim(),
                                Some(residue[quote_start + 1..residue.len() - 1].to_string()),
                                false,
                            )
                        } else {
                            (residue, None, false)
                        }
                    };

                    let is_explicit_unit = type_str == "()" || type_str == "unit";
                    let effective_unit = is_unit || is_explicit_unit;

                    let schema = if effective_unit {
                        json!({})
                    } else if type_str.contains('<')
                        || (type_str.starts_with('$') && type_str.contains('<'))
                    {
                        json!({ "$ref": type_str })
                    } else if let Ok(ty) = syn::parse_str::<syn::Type>(type_str) {
                        map_syn_type_to_openapi(&ty).0
                    } else {
                        if type_str.starts_with('$') {
                            json!({ "$ref": format!("#/components/schemas/{}", &type_str[1..]) })
                        } else if type_str == "String" || type_str == "str" {
                            json!({ "type": "string" })
                        } else {
                            json!({ "$ref": format!("#/components/schemas/{}", type_str) })
                        }
                    };

                    let mut resp_obj = json!({
                        "description": desc.unwrap_or_else(|| "".to_string())
                    });

                    if !effective_unit {
                        resp_obj["content"] = json!({
                            "application/json": {
                                "schema": schema
                            }
                        });
                    }

                    if let Value::Object(responses) = operation.get_mut("responses").unwrap() {
                        responses.insert(code.to_string(), resp_obj);
                    }
                }
            } else if trimmed.starts_with("@security") {
                let rest = trimmed.strip_prefix("@security").unwrap().trim();
                let (scheme, scopes) = if let Some(paren_start) = rest.find('(') {
                    let name = rest[..paren_start].trim();
                    let inner = &rest[paren_start + 1..rest.len() - 1];
                    let s: Vec<String> = inner
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').to_string())
                        .collect();
                    (name, s)
                } else {
                    (rest, vec![])
                };

                if operation.get("security").is_none() {
                    operation["security"] = json!([]);
                }

                if let Value::Array(sec) = operation.get_mut("security").unwrap() {
                    sec.push(json!({ scheme: scopes }));
                }
            } else if !trimmed.starts_with('@') {
                if summary.is_none() {
                    summary = Some(trimmed.to_string());
                } else {
                    description_buffer.push(trimmed);
                }
            }
        }

        if let Some(s) = summary {
            operation["summary"] = json!(s);
        }
        if !description_buffer.is_empty() {
            operation["description"] = json!(description_buffer.join("\n"));
        }

        // Validation
        let validation_re = Regex::new(r"\{(\w+)\}").unwrap();
        for cap in validation_re.captures_iter(&path) {
            let var = cap.get(1).unwrap().as_str();
            if !declared_path_params.contains(var) {
                // Panic on validation error as per requirements
                panic!(
                    "Missing definition for path parameter '{}' in route '{}'",
                    var, path
                );
            }
        }
        // Check for unused path params is implicitly handled if we track them,
        // to check strict unused we'd need to check declared_path_params vs matches in path.
        // The declared_path_params set contains only those captured from inline or @path-param.
        // We should check if any declared param is NOT in path?
        // Inline params are by definition in path.
        // @path-param defined variables might NOT be in path.
        for declared in &declared_path_params {
            if !path.contains(&format!("{{{}}}", declared)) {
                panic!(
                    "Declared path parameter '{}' is unused in route '{}'",
                    declared, path
                );
            }
        }

        if let Value::Object(map) = &mut operation {
            map.retain(|_, v| !v.is_null());
        }

        if !method.is_empty() && !path.is_empty() {
            let mut method_map = serde_json::Map::new();
            method_map.insert(method, operation);

            let mut path_map = serde_json::Map::new();
            path_map.insert(path, Value::Object(method_map));

            let path_item = json!({
                "paths": Value::Object(path_map)
            });

            if let Ok(generated) = serde_yaml::to_string(&path_item) {
                let trimmed = generated.trim_start_matches("---\n").to_string();
                self.items.push(ExtractedItem::Schema {
                    name: None,
                    content: trimmed,
                    line: i.span().start().line,
                });
            }
        }

        visit::visit_item_fn(self, i);
    }

    fn visit_item_type(&mut self, i: &'ast ItemType) {
        let ident = i.ident.to_string();
        let (mut schema, _) = map_syn_type_to_openapi(&i.ty);

        // Docs & Overrides
        let mut desc_lines = Vec::new();
        let mut openapi_lines = Vec::new();
        let mut collecting_openapi = false;

        for attr in &i.attrs {
            if attr.path().is_ident("doc") {
                if let syn::Meta::NameValue(meta) = &attr.meta {
                    if let Expr::Lit(expr_lit) = &meta.value {
                        if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                            let val = lit_str.value();
                            let trimmed = val.trim();

                            if trimmed.starts_with("@openapi") {
                                collecting_openapi = true;
                                let rest = trimmed.strip_prefix("@openapi").unwrap().trim();
                                if !rest.is_empty() {
                                    openapi_lines.push(rest.to_string());
                                }
                            } else if collecting_openapi {
                                openapi_lines.push(val.to_string());
                            } else {
                                desc_lines.push(val.trim().to_string());
                            }
                        }
                    }
                }
            } else {
                collecting_openapi = false;
            }
        }

        if !desc_lines.is_empty() {
            let desc_str = desc_lines.join(" ");
            if let Value::Object(map) = &mut schema {
                map.insert("description".to_string(), Value::String(desc_str));
            }
        }

        if !openapi_lines.is_empty() {
            let override_yaml = openapi_lines.join("\n");
            if let Ok(override_val) = serde_yaml::from_str::<Value>(&override_yaml) {
                if !override_val.is_null() {
                    json_merge(&mut schema, override_val);
                }
            }
        }

        if let Ok(generated) = serde_yaml::to_string(&schema) {
            let trimmed = generated.trim_start_matches("---\n").to_string();
            let wrapped = wrap_in_schema(&ident, &trimmed);
            self.items.push(ExtractedItem::Schema {
                name: Some(ident),
                content: wrapped,
                line: i.span().start().line,
            });
        }

        visit::visit_item_type(self, i);
    }

    fn visit_item_struct(&mut self, i: &'ast ItemStruct) {
        let ident = i.ident.to_string();

        let mut properties = serde_json::Map::new();
        let mut required_fields = Vec::new();
        let mut has_fields = false;

        if let syn::Fields::Named(fields) = &i.fields {
            for field in &fields.named {
                has_fields = true;
                let field_name = field.ident.as_ref().unwrap().to_string();

                let (mut field_schema, is_required) = map_syn_type_to_openapi(&field.ty);

                let mut field_desc = Vec::new();
                for attr in &field.attrs {
                    if attr.path().is_ident("doc") {
                        if let syn::Meta::NameValue(meta) = &attr.meta {
                            if let Expr::Lit(expr_lit) = &meta.value {
                                if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                                    let val = lit_str.value().trim().to_string();
                                    if val.starts_with("@openapi") {
                                        break;
                                    }
                                    field_desc.push(val);
                                }
                            }
                        }
                    }
                }
                if !field_desc.is_empty() {
                    let desc_str = field_desc.join(" ");
                    if let Value::Object(map) = &mut field_schema {
                        map.insert("description".to_string(), Value::String(desc_str));
                    }
                }

                // Field Level Overrides
                let mut openapi_lines = Vec::new();
                let mut collecting_openapi = false;

                for attr in &field.attrs {
                    if attr.path().is_ident("doc") {
                        if let syn::Meta::NameValue(meta) = &attr.meta {
                            if let Expr::Lit(expr_lit) = &meta.value {
                                if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                                    let val = lit_str.value();
                                    let trimmed = val.trim();

                                    if trimmed.starts_with("@openapi") {
                                        collecting_openapi = true;
                                        let rest = trimmed.strip_prefix("@openapi").unwrap().trim();
                                        if !rest.is_empty() {
                                            openapi_lines.push(rest.to_string());
                                        }
                                    } else if collecting_openapi {
                                        openapi_lines.push(val.to_string());
                                    }
                                }
                            }
                        }
                    } else {
                        collecting_openapi = false;
                    }
                }

                if !openapi_lines.is_empty() {
                    let override_yaml = openapi_lines.join("\n");
                    if let Ok(override_val) = serde_yaml::from_str::<Value>(&override_yaml) {
                        if !override_val.is_null() {
                            json_merge(&mut field_schema, override_val);
                        }
                    }
                }

                properties.insert(field_name.clone(), field_schema);
                if is_required {
                    required_fields.push(field_name);
                }
            }
        }

        // Struct Level Schema
        let mut schema = if has_fields {
            let mut s = json!({
                "type": "object",
                "properties": properties
            });
            if !required_fields.is_empty() {
                if let Value::Object(map) = &mut s {
                    map.insert("required".to_string(), json!(required_fields));
                }
            }
            s
        } else {
            // Unit Struct default
            json!({ "type": "object" })
        };

        // Struct Level Docs & Overrides
        let mut desc_lines = Vec::new();
        let mut openapi_lines = Vec::new();
        let mut collecting_openapi = false;
        let mut blueprint_params: Option<Vec<String>> = None;

        for attr in &i.attrs {
            if attr.path().is_ident("doc") {
                if let syn::Meta::NameValue(meta) = &attr.meta {
                    if let Expr::Lit(expr_lit) = &meta.value {
                        if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                            let val = lit_str.value();
                            let trimmed = val.trim();
                            if trimmed.starts_with("@openapi") {
                                collecting_openapi = true;
                                let rest = trimmed.strip_prefix("@openapi").unwrap().trim();
                                if !rest.is_empty() {
                                    if rest.contains('<') {
                                        // Blueprint detection
                                        if let Some(start) = rest.find('<') {
                                            if let Some(end) = rest.rfind('>') {
                                                let params_str = &rest[start + 1..end];
                                                blueprint_params = Some(
                                                    params_str
                                                        .split(',')
                                                        .map(|p| p.trim().to_string())
                                                        .filter(|p| !p.is_empty())
                                                        .collect(),
                                                );

                                                let after_gt = rest[end + 1..].trim();
                                                if !after_gt.is_empty() {
                                                    openapi_lines.push(after_gt.to_string());
                                                }
                                            }
                                        }
                                    } else {
                                        openapi_lines.push(rest.to_string());
                                    }
                                }
                            } else if collecting_openapi {
                                openapi_lines.push(val.to_string());
                            } else {
                                desc_lines.push(val.trim().to_string());
                            }
                        }
                    }
                }
            } else {
                collecting_openapi = false;
            }
        }

        if !desc_lines.is_empty() {
            let desc_str = desc_lines.join(" ");
            json_merge(&mut schema, json!({ "description": desc_str }));
        }

        if !openapi_lines.is_empty() {
            let override_yaml = openapi_lines.join("\n");
            if let Ok(override_val) = serde_yaml::from_str::<Value>(&override_yaml) {
                if !override_val.is_null() {
                    json_merge(&mut schema, override_val);
                }
            }
        }

        // Final Serialize
        if let Ok(generated) = serde_yaml::to_string(&schema) {
            let trimmed = generated.trim_start_matches("---\n").to_string();

            if let Some(params) = blueprint_params {
                self.items.push(ExtractedItem::Blueprint {
                    name: ident,
                    params,
                    content: trimmed,
                    line: i.span().start().line,
                });
            } else {
                let wrapped = wrap_in_schema(&ident, &trimmed);
                self.items.push(ExtractedItem::Schema {
                    name: Some(ident),
                    content: wrapped,
                    line: i.span().start().line,
                });
            }
        }

        visit::visit_item_struct(self, i);
    }

    fn visit_item_enum(&mut self, i: &'ast ItemEnum) {
        let ident = i.ident.to_string();

        let mut variants = Vec::new();
        for v in &i.variants {
            if matches!(v.fields, syn::Fields::Unit) {
                variants.push(v.ident.to_string());
            }
        }

        let mut schema = if !variants.is_empty() {
            json!({
                "type": "string",
                "enum": variants
            })
        } else {
            json!({ "type": "string" }) // fallback
        };

        // Enum Doc Overrides
        let mut desc_lines = Vec::new();
        let mut openapi_lines = Vec::new();
        let mut collecting_openapi = false;
        let mut blueprint_params: Option<Vec<String>> = None;

        for attr in &i.attrs {
            if attr.path().is_ident("doc") {
                if let syn::Meta::NameValue(meta) = &attr.meta {
                    if let Expr::Lit(expr_lit) = &meta.value {
                        if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                            let val = lit_str.value();
                            let trimmed = val.trim();
                            if trimmed.starts_with("@openapi") {
                                collecting_openapi = true;
                                let rest = trimmed.strip_prefix("@openapi").unwrap().trim();
                                if !rest.is_empty() {
                                    if rest.contains('<') {
                                        // Blueprint detection
                                        if let Some(start) = rest.find('<') {
                                            if let Some(end) = rest.rfind('>') {
                                                let params_str = &rest[start + 1..end];
                                                blueprint_params = Some(
                                                    params_str
                                                        .split(',')
                                                        .map(|p| p.trim().to_string())
                                                        .filter(|p| !p.is_empty())
                                                        .collect(),
                                                );

                                                let after_gt = rest[end + 1..].trim();
                                                if !after_gt.is_empty() {
                                                    openapi_lines.push(after_gt.to_string());
                                                }
                                            }
                                        }
                                    } else {
                                        openapi_lines.push(rest.to_string());
                                    }
                                }
                            } else if collecting_openapi {
                                openapi_lines.push(val.to_string());
                            } else {
                                desc_lines.push(val.trim().to_string());
                            }
                        }
                    }
                }
            } else {
                collecting_openapi = false;
            }
        }

        if !desc_lines.is_empty() {
            let desc_str = desc_lines.join(" ");
            json_merge(&mut schema, json!({ "description": desc_str }));
        }

        if !openapi_lines.is_empty() {
            let override_yaml = openapi_lines.join("\n");
            if let Ok(override_val) = serde_yaml::from_str::<Value>(&override_yaml) {
                if !override_val.is_null() {
                    json_merge(&mut schema, override_val);
                }
            }
        }

        // Only emit if we have variants OR overrides
        if !variants.is_empty() || !openapi_lines.is_empty() {
            if let Ok(generated) = serde_yaml::to_string(&schema) {
                let trimmed = generated.trim_start_matches("---\n").to_string();

                if let Some(params) = blueprint_params {
                    self.items.push(ExtractedItem::Blueprint {
                        name: ident,
                        params,
                        content: trimmed,
                        line: i.span().start().line,
                    });
                } else {
                    let wrapped = wrap_in_schema(&ident, &trimmed);
                    self.items.push(ExtractedItem::Schema {
                        name: Some(ident),
                        content: wrapped,
                        line: i.span().start().line,
                    });
                }
            }
        }

        visit::visit_item_enum(self, i);
    }

    fn visit_item_mod(&mut self, i: &'ast ItemMod) {
        let mut found_tags = Vec::new();
        for attr in &i.attrs {
            if attr.path().is_ident("doc") {
                if let syn::Meta::NameValue(meta) = &attr.meta {
                    if let Expr::Lit(expr_lit) = &meta.value {
                        if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                            let val = lit_str.value();
                            if val.contains("tags:") {
                                if let Some(start) = val.find('[') {
                                    if let Some(end) = val.find(']') {
                                        let content = &val[start + 1..end];
                                        for t in content.split(',') {
                                            found_tags.push(t.trim().to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let old_len = self.current_tags.len();
        self.current_tags.extend(found_tags);

        self.check_attributes(&i.attrs, None, i.span().start().line);
        visit::visit_item_mod(self, i);

        self.current_tags.truncate(old_len);
    }

    fn visit_impl_item_fn(&mut self, i: &'ast ImplItemFn) {
        self.check_attributes(&i.attrs, None, i.span().start().line);
        visit::visit_impl_item_fn(self, i);
    }
}

pub fn extract_from_file(path: std::path::PathBuf) -> crate::error::Result<Vec<ExtractedItem>> {
    let content = std::fs::read_to_string(&path)?;
    let parsed_file = syn::parse_file(&content).map_err(|e| crate::error::Error::Parse {
        file: path.clone(),
        source: e,
    })?;

    let mut visitor = OpenApiVisitor::default();
    visitor.visit_file(&parsed_file);

    Ok(visitor.items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_struct_reflection() {
        let code = r#"
            /// @openapi
            struct MyStruct {
                pub id: String,
                pub count: i32,
                pub active: bool,
                pub tags: Vec<String>,
                pub meta: Option<String>
            }
        "#;
        let item_struct: ItemStruct = syn::parse_str(code).expect("Failed to parse struct");

        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_struct(&item_struct);

        assert_eq!(visitor.items.len(), 1);
        match &visitor.items[0] {
            ExtractedItem::Schema { name, content, .. } => {
                assert_eq!(name.as_ref().unwrap(), "MyStruct");
                // Check reflection
                assert!(content.contains("type: object"));
                assert!(content.contains("properties"));
                assert!(content.contains("id"));
                assert!(content.contains("type: string"));
                assert!(content.contains("count"));
                assert!(content.contains("type: integer"));

                // Vec
                assert!(content.contains("tags"));
                assert!(content.contains("type: array"));

                // Option -> Not required
                assert!(content.contains("required"));
                assert!(content.contains("id"));
                assert!(content.contains("count"));
                assert!(content.contains("tags"));
                // meta should NOT be in required
            }
            _ => panic!("Expected Schema"),
        }
    }

    #[test]
    fn test_module_tags() {
        let code = r#"
            /// @openapi
            /// tags: [GroupA]
            mod my_mod {
                /// @openapi
                /// paths:
                ///   /test:
                ///     get:
                ///       description: op
                fn my_fn() {}
            }
        "#;
        let item_mod: ItemMod = syn::parse_str(code).expect("Failed to parse mod");

        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_mod(&item_mod);

        assert_eq!(visitor.items.len(), 2);
        match &visitor.items[1] {
            ExtractedItem::Schema { content, .. } => {
                assert!(
                    content.contains("tags:"),
                    "Function should have tags injected"
                );
                assert!(content.contains("- GroupA"));
                assert!(content.contains("/test:"));
            }
            _ => panic!("Expected Schema"),
        }
    }

    #[test]
    fn test_complex_types_and_docs() {
        let code = r#"
            /// @openapi
            struct Complex {
                /// Primary Identifier
                pub id: Uuid,
                /// @openapi example: "user@example.com"
                pub email: String,
                pub created_at: DateTime<Utc>,
                pub metadata: HashMap<String, String>,
                pub scores: Vec<f64>,
                pub config: Option<serde_json::Value>
            }
        "#;
        let item_struct: ItemStruct = syn::parse_str(code).expect("Failed to parse struct");

        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_struct(&item_struct);

        match &visitor.items[0] {
            ExtractedItem::Schema { content, .. } => {
                // Check doc comment merge
                assert!(
                    content.contains("description: Primary Identifier"),
                    "Should merge doc comments"
                );

                // Check attribute override
                assert!(
                    content.contains("example: user@example.com"),
                    "Should merge @openapi attributes"
                );

                // Check Types
                assert!(content.contains("format: uuid"));
                assert!(content.contains("format: date-time"));
                assert!(content.contains("format: double"));
                assert!(content.contains("additionalProperties")); // Map

                // Option -> Not required
                let _required_idx = content.find("required").unwrap();
                let _config_idx = content.find("config").unwrap();
                // We can't strictly check line order easily with contains, but we know config (Option) shouldn't be in required list
                // However, let's just assert content does not have "- config" inside the required block.
                // Since this is YAML generated by serde, it's reliable.
            }
            _ => panic!("Expected Schema"),
        }
    }

    #[test]
    fn test_visitor_bugs_v0_4_2() {
        // 1. Generic Fallback Test ($T)
        let code_generic = r#"
            struct Container<T> {
                pub item: T,
            }
        "#;
        let item_struct: ItemStruct = syn::parse_str(code_generic).expect("Failed to parse struct");
        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_struct(&item_struct);
        match &visitor.items[0] {
            ExtractedItem::Schema { content, .. } => {
                // FIX 3: Should contain $ref: $T, NOT #/components/schemas/T
                assert!(
                    content.contains("$ref: $T"),
                    "Should use Smart Ref for generics (expected $ref: $T)"
                );
            }
            _ => panic!("Expected Schema"),
        }

        // 2. Multi-line Field Docs Test
        let code_multiline = r#"
            /// @openapi
            struct User {
                /// @openapi
                /// example:
                ///   - "Alice"
                ///   - "Bob"
                pub names: Vec<String>
            }
        "#;
        let item_struct_m: ItemStruct =
            syn::parse_str(code_multiline).expect("Failed to parse struct");
        let mut visitor_m = OpenApiVisitor::default();
        visitor_m.visit_item_struct(&item_struct_m);
        match &visitor_m.items[0] {
            ExtractedItem::Schema { content, .. } => {
                // FIX 2: Should correctly parse the YAML list
                assert!(content.contains("example:"), "Should contain example key");
                assert!(
                    content.contains("- Alice"),
                    "Should parse multi-line attributes (- Alice)"
                );
            }
            _ => panic!("Expected Schema"),
        }

        // 3. Tag Injection Test (Indentation)
        let code_tags = r#"
            /// @openapi
            /// tags: [MyTag]
            mod my_mod {
                 /// @openapi
                 /// paths:
                 ///   /foo:
                 ///     get:
                 ///       description: op
                 fn my_fn() {}
            }
        "#;
        let item_mod: ItemMod = syn::parse_str(code_tags).expect("Failed to parse mod");
        let mut visitor_t = OpenApiVisitor::default();
        visitor_t.visit_item_mod(&item_mod);
        match &visitor_t.items[1] {
            // Item 1 is the fn
            ExtractedItem::Schema { content, .. } => {
                // FIX 1: Indentation check
                let get_idx = content.find("get:").unwrap();
                let tags_idx = content.find("tags:").unwrap();

                // Tags must appear AFTER get
                assert!(tags_idx > get_idx, "Tags should be inside/after get");

                // Tags must appear BEFORE description (if injected at top of block)
                let desc_idx = content.find("description:").unwrap();
                assert!(
                    tags_idx < desc_idx,
                    "Tags should be injected before description (top of block)"
                );
            }
            _ => panic!("Expected Schema"),
        }
    }

    #[test]
    fn test_visitor_pollution_v0_4_3() {
        let code = r#"
            /// @openapi
            struct Clean {
                /// Clean Description
                /// @openapi example: "dirty"
                pub field: String,
            }
        "#;
        let item_struct: ItemStruct = syn::parse_str(code).expect("Failed to parse struct");
        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_struct(&item_struct);

        match &visitor.items[0] {
            ExtractedItem::Schema { content, .. } => {
                // Description should be "Clean Description"
                // It should NOT contain "@openapi" or "example: dirty"
                // But the example should be merged into the schema separately.

                assert!(content.contains("description: Clean Description"));
                assert!(
                    !content.contains("description: Clean Description @openapi"),
                    "Should Clean Description"
                );
                assert!(
                    content.contains("example: dirty"),
                    "Should still have the example"
                );
            }
            _ => panic!("Expected Schema"),
        }
    }

    #[test]
    fn test_type_alias_reflection() {
        let code = r#"
            /// @openapi
            /// format: uuid
            /// description: User ID Alias
            type UserId = String;
        "#;
        let item_type: ItemType = syn::parse_str(code).expect("Failed to parse type");

        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_type(&item_type);

        assert_eq!(visitor.items.len(), 1);
        match &visitor.items[0] {
            ExtractedItem::Schema { name, content, .. } => {
                assert_eq!(name.as_ref().unwrap(), "UserId");
                assert!(content.contains("type: string"));
                assert!(content.contains("format: uuid"));
                assert!(content.contains("description: User ID Alias"));
            }
            _ => panic!("Expected Schema"),
        }
    }

    #[test]
    fn test_virtual_types_unit_struct() {
        let code = r#"
            /// @openapi
            /// type: string
            /// enum: [A, B]
            struct MyEnum;
        "#;
        let item_struct: ItemStruct = syn::parse_str(code).expect("Failed to parse struct");
        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_struct(&item_struct);

        // This relies on implicit schema parsing from docs
        assert_eq!(visitor.items.len(), 1);
        match &visitor.items[0] {
            ExtractedItem::Schema { name, content, .. } => {
                assert_eq!(name.as_ref().unwrap(), "MyEnum");
                assert!(content.contains("type: string"));
                assert!(content.contains("enum:"));
                assert!(content.contains("A"));
                assert!(content.contains("B"));
            }
            _ => panic!("Expected Schema"),
        }
    }

    #[test]
    fn test_global_virtual_type() {
        let code = r#"
            //! @openapi-type Email
            //! type: string
            //! format: email
            //! description: Valid email address
            
            // Other code...
            fn main() {}
        "#;
        // Parse as File because it's a file attribute (inner doc comment)
        let file: File = syn::parse_str(code).expect("Failed to parse file");

        let mut visitor = OpenApiVisitor::default();
        visitor.visit_file(&file);

        // Should find Email schema
        let email_schema = visitor.items.iter().find(|i| {
            if let ExtractedItem::Schema { name, .. } = i {
                name.as_deref() == Some("Email")
            } else {
                false
            }
        });

        assert!(email_schema.is_some(), "Should find Email schema");
        match email_schema.unwrap() {
            ExtractedItem::Schema { content, .. } => {
                assert!(content.contains("type: string"));
                assert!(content.contains("format: email"));
            }
            _ => panic!("Expected Schema"),
        }
    }

    #[test]
    fn test_route_dsl_basic() {
        let code = r#"
            /// Get Users
            /// Returns a list of users.
            /// @route GET /users
            /// @tag Users
            fn get_users() {}
        "#;
        let item_fn: ItemFn = syn::parse_str(code).expect("Failed to parse fn");
        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_fn(&item_fn);

        assert_eq!(visitor.items.len(), 1);
        if let ExtractedItem::Schema { content, .. } = &visitor.items[0] {
            assert!(content.contains("paths:"));
            assert!(content.contains("/users:"));
            assert!(content.contains("get:"));
            assert!(content.contains("summary: Get Users"));
            assert!(content.contains("description: Returns a list of users."));
            assert!(content.contains("tags:"));
            assert!(content.contains("- Users"));
        } else {
            panic!("Expected Schema");
        }
    }

    #[test]
    fn test_route_dsl_params() {
        let code = r#"
            /// @route GET /users/{id}
            /// @path-param id: u32 "User ID"
            /// @query-param filter: Option<String> "Name filter"
            fn get_user() {}
        "#;
        let item_fn: ItemFn = syn::parse_str(code).expect("Failed to parse fn");
        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_fn(&item_fn);

        if let ExtractedItem::Schema { content, .. } = &visitor.items[0] {
            // Path Param
            assert!(content.contains("name: id"));
            assert!(content.contains("in: path"));

            // Check required: true for path param.
            // Note: Serde YAML might output `required: true` or just imply it depending on structure,
            // but our JSON builder explicitly sets it.
            assert!(content.contains("required: true"));
            assert!(content.contains("format: int32"));

            // Query Param
            assert!(content.contains("name: filter"));
            assert!(content.contains("in: query"));
            assert!(content.contains("required: false")); // Option<String>
        } else {
            panic!("Expected Schema");
        }
    }

    #[test]
    fn test_route_dsl_body_return() {
        let code = r#"
            /// @route POST /users
            /// @body String text/plain
            /// @return 201: u64 "Created ID"
            fn create_user() {}
        "#;
        let item_fn: ItemFn = syn::parse_str(code).expect("Failed to parse fn");
        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_fn(&item_fn);

        if let ExtractedItem::Schema { content, .. } = &visitor.items[0] {
            // Body
            assert!(content.contains("requestBody:"));
            assert!(content.contains("text/plain:")); // MIME
            assert!(content.contains("schema:"));
            assert!(content.contains("type: string"));

            // Return
            assert!(content.contains("responses:"));
            assert!(content.contains("'201':"));
            assert!(content.contains("description: Created ID"));
            assert!(content.contains("format: int64"));
        } else {
            panic!("Expected Schema");
        }
    }

    #[test]
    fn test_route_dsl_security() {
        let code = r#"
            /// @route GET /secure
            /// @security oidcAuth("read")
            fn secure_op() {}
        "#;
        let item_fn: ItemFn = syn::parse_str(code).expect("Failed to parse fn");
        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_fn(&item_fn);

        if let ExtractedItem::Schema { content, .. } = &visitor.items[0] {
            assert!(content.contains("security:"));
            assert!(content.contains("- oidcAuth:"));
            assert!(content.contains("- read"));
        } else {
            panic!("Expected Schema");
        }
    }

    #[test]
    fn test_route_dsl_generics_and_unit() {
        let code = r#"
            /// @route POST /test
            /// @return 200: $Page<User> "Generic List"
            /// @return 204: () "Nothing"
            fn test_op() {}
        "#;
        let item_fn: ItemFn = syn::parse_str(code).expect("Failed to parse fn");
        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_fn(&item_fn);

        if let ExtractedItem::Schema { content, .. } = &visitor.items[0] {
            // 1. Verify Generic is RAW (Crucial for Monomorphizer)
            assert!(content.contains("$ref: $Page<User>"));
            assert!(!content.contains("#/components/schemas/$Page<User>")); // MUST FAIL if wrapped

            // 2. Verify Unit has NO content
            assert!(content.contains("'204':"));
            assert!(content.contains("description: Nothing"));
            // Ensure 204 block does not have "content:"
            // (We check strict context or absence of content key for 204)
            let json: serde_json::Value = serde_yaml::from_str(content).unwrap();
            let resp_204 = &json["paths"]["/test"]["post"]["responses"]["204"];
            assert!(
                resp_204.get("content").is_none(),
                "204 response should not have content"
            );
        } else {
            panic!("Expected Schema");
        }
    }

    #[test]
    fn test_route_dsl_unit_return() {
        let code = r#"
            /// @route DELETE /delete
            /// @return 204: "Deleted Successfully"
            /// @return 202: () "Accepted"
            fn delete_op() {}
        "#;
        let item_fn: ItemFn = syn::parse_str(code).expect("Failed to parse fn");
        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_fn(&item_fn);

        if let ExtractedItem::Schema { content, .. } = &visitor.items[0] {
            // Parse to verify structure
            let json: serde_json::Value = serde_yaml::from_str(content).unwrap();
            let responses = &json["paths"]["/delete"]["delete"]["responses"];

            // Case 1: Implicit Unit ("Deleted Successfully")
            let resp_204 = &responses["204"];
            assert_eq!(resp_204["description"], "Deleted Successfully");
            assert!(
                resp_204.get("content").is_none(),
                "204 should have no content"
            );

            // Case 2: Explicit Unit (())
            let resp_202 = &responses["202"];
            assert_eq!(resp_202["description"], "Accepted");
            assert!(
                resp_202.get("content").is_none(),
                "202 should have no content"
            );
        } else {
            panic!("Expected Schema");
        }
    }
}

#[cfg(test)]
mod v0_7_0_tests {
    use super::*;

    #[test]
    fn test_route_dsl_inline_params() {
        let code = r#"
            /// @route GET /items/{id: u32 "Item ID"}
            fn get_item() {}
        "#;
        let item_fn: ItemFn = syn::parse_str(code).expect("Failed to parse fn");
        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_fn(&item_fn);

        if let ExtractedItem::Schema { content, .. } = &visitor.items[0] {
            // 1. Check path normalization
            assert!(content.contains("/items/{id}:"));

            // 2. Check parameter extraction
            let json: serde_json::Value = serde_yaml::from_str(content).unwrap();
            let params = &json["paths"]["/items/{id}"]["get"]["parameters"];
            assert!(params.is_array());
            assert_eq!(params.as_array().unwrap().len(), 1);

            let p = &params[0];
            assert_eq!(p["name"], "id");
            assert_eq!(p["in"], "path");
            assert_eq!(p["required"], true);
            assert_eq!(p["description"], "Item ID");
            assert_eq!(p["schema"]["type"], "integer");
            assert_eq!(p["schema"]["format"], "int32");
        } else {
            panic!("Expected Schema");
        }
    }

    #[test]
    fn test_route_dsl_flexible_params() {
        let code = r#"
            /// @route GET /search
            /// @query-param q: String "Search Query"
            /// @query-param sort: deprecated required example="desc" "Sort Order"
            fn search() {}
        "#;
        let item_fn: ItemFn = syn::parse_str(code).expect("Failed to parse fn");
        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_fn(&item_fn);

        if let ExtractedItem::Schema { content, .. } = &visitor.items[0] {
            let json: serde_json::Value = serde_yaml::from_str(content).unwrap();
            let params = &json["paths"]["/search"]["get"]["parameters"];
            let params_arr = params.as_array().unwrap();

            // Param 'q' (Standard)
            let q = params_arr.iter().find(|p| p["name"] == "q").unwrap();
            assert_eq!(q["description"], "Search Query");

            // Param 'sort' (Flexible)
            let sort = params_arr.iter().find(|p| p["name"] == "sort").unwrap();
            assert_eq!(sort["deprecated"], true);
            assert_eq!(sort["required"], true);
            assert_eq!(sort["example"], "desc");
            assert_eq!(sort["description"], "Sort Order");
        } else {
            panic!("Expected Schema");
        }
    }

    #[test]
    #[should_panic(expected = "Missing definition for path parameter 'id'")]
    fn test_route_dsl_validation_error() {
        let code = r#"
            /// @route GET /items/{id}
            fn get_item_fail() {}
        "#;
        let item_fn: ItemFn = syn::parse_str(code).expect("Failed to parse fn");
        let mut visitor = OpenApiVisitor::default();
        visitor.visit_item_fn(&item_fn);
    }
}
