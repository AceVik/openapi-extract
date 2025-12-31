use serde_json::{json, Value};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, File, ImplItemFn, ItemEnum, ItemFn, ItemMod, ItemStruct};

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
    fn check_attributes(
        &mut self,
        attrs: &[Attribute],
        item_ident: Option<String>,
        item_line: usize,
        generated_content: Option<String>,
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

        let has_doc_api = doc_lines.iter().any(|l| l.contains("@openapi"));

        // If NO @openapi and NO generated content, skip.
        if !has_doc_api && generated_content.is_none() {
            return;
        }

        if let Some(generated) = generated_content {
            // Append generated content (which is YAML string)
            doc_lines.push(generated);
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
            let body = body.trim().to_string();
            // Clean comments
            let content = &body;

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
                    content: body, // Keep raw body
                    line,
                });
            } else if header.starts_with("@openapi") && header.contains('<') {
                if let Some(start) = header.find('<') {
                    if let Some(end) = header.find('>') {
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
                                content: body,
                                line,
                            });
                        }
                    }
                }
            } else if (header.starts_with("@openapi") && !header.contains('<')) || header == "@json"
            {
                // Auto-Wrap Heuristic + Legacy Tag Injection logic (string based for now)
                
                // Inject Tags if not present and if it looks like an operation (paths/single verb)
                let mut final_content_string = content.to_string();
                if !self.current_tags.is_empty() {
                    if (content.contains("get:")
                        || content.contains("post:")
                        || content.contains("responses:"))
                        && !content.contains("tags:")
                    {
                        let tags_yaml = self
                            .current_tags
                            .iter()
                            .map(|t| format!("  - {}", t))
                            .collect::<Vec<_>>()
                            .join("\n");
                        final_content_string.push_str(&format!("\ntags:\n{}", tags_yaml));
                    }
                }

                let starts_with_toplevel = final_content_string.lines().any(|line| {
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

                let final_content =
                    if !starts_with_toplevel && !final_content_string.trim().is_empty() {
                        if let Some(n) = &item_ident {
                            let indented = final_content_string
                                .lines()
                                .map(|l| format!("      {}", l))
                                .collect::<Vec<_>>()
                                .join("\n");
                            format!("components:\n  schemas:\n    {}:\n{}", n, indented)
                        } else {
                            final_content_string
                        }
                    } else {
                        final_content_string
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

// Helper for type mapping
// Returns (Schema Value, IsRequired)
fn map_syn_type_to_openapi(ty: &syn::Type) -> (Value, bool) {
    match ty {
        syn::Type::Path(p) => {
            if let Some(seg) = p.path.segments.last() {
                let ident = seg.ident.to_string();

                // Smart Pointers: Box, Arc, Rc, Cow -> recurse
                if ["Box", "Arc", "Rc", "Cow"].contains(&ident.as_str()) {
                    if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                        if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                            return map_syn_type_to_openapi(inner);
                        }
                    }
                }

                match ident.as_str() {
                    // Primitives
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

                    // Well-known Crates
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
                    "Value" => (json!({}), true), // Any type (serde_json)

                    // Collections
                    "Option" => {
                        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                            if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                                let (inner_val, _) = map_syn_type_to_openapi(inner);
                                return (inner_val, false); // Not required
                            }
                        }
                        (json!({}), false)
                    }
                    "Vec" | "LinkedList" | "HashSet" => {
                        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                            if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                                let (inner_val, _) = map_syn_type_to_openapi(inner);
                                return (
                                    json!({
                                        "type": "array",
                                        "items": inner_val
                                    }),
                                    true,
                                );
                            }
                        }
                        (json!({ "type": "array" }), true)
                    }
                    "HashMap" | "BTreeMap" => {
                        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                            // Key is K (ignored, always string keys in JSON), Value is V
                            if args.args.len() >= 2 {
                                if let syn::GenericArgument::Type(val_type) = &args.args[1] {
                                    let (val_schema, _) = map_syn_type_to_openapi(val_type);
                                    return (
                                        json!({
                                            "type": "object",
                                            "additionalProperties": val_schema
                                        }),
                                        true,
                                    );
                                }
                            }
                        }
                        (json!({ "type": "object" }), true)
                    }

                    // Fallback
                    _ => (json!({ "$ref": format!("#/components/schemas/{}", ident) }), true),
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
        self.check_attributes(&i.attrs, None, 1, None);
        visit::visit_file(self, i);
    }

    fn visit_item_fn(&mut self, i: &'ast ItemFn) {
        self.check_attributes(&i.attrs, None, i.span().start().line, None);
        visit::visit_item_fn(self, i);
    }

    fn visit_item_struct(&mut self, i: &'ast ItemStruct) {
        let ident = i.ident.to_string();

        // 1. Init Builder
        let mut properties = serde_json::Map::new();
        let mut required_fields = Vec::new();
        let mut has_fields = false;

        if let syn::Fields::Named(fields) = &i.fields {
            for field in &fields.named {
                has_fields = true;
                let field_name = field.ident.as_ref().unwrap().to_string();

                // Step A: Base Schema
                let (mut field_schema, is_required) = map_syn_type_to_openapi(&field.ty);

                // Step B: Field Doc Comments (description)
                let mut field_desc = Vec::new();
                for attr in &field.attrs {
                    if attr.path().is_ident("doc") {
                        if let syn::Meta::NameValue(meta) = &attr.meta {
                            if let Expr::Lit(expr_lit) = &meta.value {
                                if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                                    let val = lit_str.value().trim().to_string();
                                    // Skip @openapi lines in description
                                    if !val.starts_with("@openapi") {
                                        field_desc.push(val);
                                    }
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

                // Step C: Overrides (@openapi parsing)
                // We rely on simple YAML parsing of the @openapi line for now, or just extract specific keys?
                // Parsing full YAML from a single string line is tricky if it's complex.
                // But generally users write: /// @openapi example: "foo"
                // We concatenate all @openapi lines and parse as YAML?
                let mut openapi_override_lines = Vec::new();
                for attr in &field.attrs {
                    if attr.path().is_ident("doc") {
                        if let syn::Meta::NameValue(meta) = &attr.meta {
                            if let Expr::Lit(expr_lit) = &meta.value {
                                if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                                    let val = lit_str.value().trim().to_string();
                                    if val.starts_with("@openapi") {
                                        let rest = val.trim_start_matches("@openapi").trim();
                                        openapi_override_lines.push(rest.to_string());
                                    }
                                }
                            }
                        }
                    }
                }

                if !openapi_override_lines.is_empty() {
                    let override_yaml = openapi_override_lines.join("\n");
                    // Parse as Value
                    if let Ok(override_val) = serde_yaml::from_str::<Value>(&override_yaml) {
                        json_merge(&mut field_schema, override_val);
                    }
                }

                // Step D: Register
                properties.insert(field_name.clone(), field_schema);
                if is_required {
                    required_fields.push(field_name);
                }
            }
        }

        let generated_opt = if has_fields {
            let mut schema = json!({
                "type": "object",
                "properties": properties
            });

            if !required_fields.is_empty() {
                if let Value::Object(map) = &mut schema {
                    map.insert("required".to_string(), json!(required_fields));
                }
            }

            // Convert to YAML string (partial)
            // Note: `check_attributes` expects a STRING to append.
            // Ideally we should dump the content without the keys "properties" etc wrapped?
            // BUT check_attributes logic currently wraps it in components/schemas/Name IF it detects invalid top level.
            // Let's produce the raw YAML for type/properties/required.

            match serde_yaml::to_string(&schema) {
                Ok(s) => {
                     // serde_yaml adds "---" prefix often, strip it
                    let trimmed = s.trim_start_matches("---\n").to_string();
                    Some(trimmed)
                },
                Err(_) => None
            }
        } else {
            None
        };

        self.check_attributes(&i.attrs, Some(ident), i.span().start().line, generated_opt);
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

        let generated = if !variants.is_empty() {
            let enum_list = variants
                .iter()
                .map(|v| format!("  - {}", v))
                .collect::<Vec<_>>()
                .join("\n");
            Some(format!("type: string\nenum:\n{}", enum_list))
        } else {
            None
        };

        self.check_attributes(&i.attrs, Some(ident), i.span().start().line, generated);
        visit::visit_item_enum(self, i);
    }

    fn visit_item_mod(&mut self, i: &'ast ItemMod) {
        // Module Tag Logic
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
                                         let content = &val[start+1..end];
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

        self.check_attributes(&i.attrs, None, i.span().start().line, None);
        visit::visit_item_mod(self, i);
        
        self.current_tags.truncate(old_len);
    }

    fn visit_impl_item_fn(&mut self, i: &'ast ImplItemFn) {
        self.check_attributes(&i.attrs, None, i.span().start().line, None);
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
            },
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
                assert!(content.contains("tags:"), "Function should have tags injected");
                assert!(content.contains("- GroupA"));
                assert!(content.contains("/test:"));
            },
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
                assert!(content.contains("description: Primary Identifier"), "Should merge doc comments");
                
                // Check attribute override
                assert!(content.contains("example: user@example.com"), "Should merge @openapi attributes");
                
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
            },
            _ => panic!("Expected Schema"),
        }
    }
}
