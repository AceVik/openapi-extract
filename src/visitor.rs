use syn::visit::{self, Visit};
use syn::{Attribute, Expr, File, ImplItemFn, ItemEnum, ItemFn, ItemMod, ItemStruct};

/// Extracted item type
#[derive(Debug)]
pub enum ExtractedItem {
    /// Standard @openapi body
    Schema {
        name: Option<String>,
        content: String,
    },
    /// @openapi-fragment Name(args...)
    Fragment {
        name: String,
        params: Vec<String>,
        content: String,
    },
    /// @openapi<T, U>
    Blueprint {
        name: String,
        params: Vec<String>,
        content: String,
    },
}

#[derive(Default)]
pub struct OpenApiVisitor {
    pub items: Vec<ExtractedItem>,
}

impl OpenApiVisitor {
    fn check_attributes(&mut self, attrs: &[Attribute], item_ident: Option<String>) {
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

        if doc_lines.is_empty() {
            return;
        }

        let full_doc = doc_lines.join("\n");
        // We process the raw block. We need to split by signatures if multiple present.
        // Or scanner/preprocessor handles it?
        // "Multi-Definition: A single doc block can contain multiple fragments. The scanner must split the block by the @openapi-fragment keyword."

        self.parse_doc_block(&full_doc, item_ident);
    }

    fn parse_doc_block(&mut self, doc: &str, item_ident: Option<String>) {
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

        // Split by keywords?
        // We look for logic lines starting with @openapi...
        // This is complex regex logic.
        // For v0.2.0 let's handle:
        // 1. @openapi-fragment Name\nBody
        // 2. @openapi<T>\nBody
        // 3. @openapi\nBody

        // We can check if the block *starts* with one of these.
        // If "Multiple fragments per block" is required, we need to split string indices.

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
                // JSON start without header? Treat as unnamed schema extension?
                // Or treat as part of previous if it exists?
                // For now, treat as new unnamed section
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
            if header.starts_with("@openapi-fragment") {
                // Parse Name from "@openapi-fragment MyFrag(a,b)"
                let rest = header.strip_prefix("@openapi-fragment").unwrap().trim();
                // Simple parse: take until first space or paren?
                // Name(args)
                if let Some(idx) = rest.find('(') {
                    let name = rest[..idx].trim().to_string();
                    let params_str = rest[idx + 1..].trim_end_matches(')');
                    let params: Vec<String> = params_str
                        .split(',')
                        .map(|p| p.trim().to_string())
                        .filter(|p| !p.is_empty())
                        .collect();

                    self.items.push(ExtractedItem::Fragment {
                        name,
                        params,
                        content: body,
                    });
                } else {
                    self.items.push(ExtractedItem::Fragment {
                        name: rest.to_string(),
                        params: Vec::new(),
                        content: body,
                    });
                }
            } else if header.starts_with("@openapi") && header.contains('<') {
                // Blueprint
                // Handle optional space: @openapi <T> or @openapi<T>
                // We use finding '<' as the prompt.

                // Header format: @openapi<T, U>
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
                            });
                        }
                    }
                }
            } else if (header.starts_with("@openapi") && !header.contains('<')) || header == "@json"
            {
                // Auto-Wrap Heuristic
                // body has the content, already cleaned/merged.
                let content = &body;

                // Auto-Wrap Heuristic
                // If content does NOT start with top-level keys, wrap it.
                // Keys: openapi, info, paths, components, tags, servers, security
                // Simple check by tokenizing or simple textual check.
                // We assume clean YAML structure.
                let starts_with_toplevel = content.lines().any(|line| {
                    let trimmed = line.trim();
                    if trimmed.starts_with("#") {
                        return false;
                    } // skip comments
                    if let Some(key) = trimmed.split(':').next() {
                        match key.trim() {
                            "openapi" | "info" | "paths" | "components" | "tags" | "servers"
                            | "security" => true,
                            _ => false,
                        }
                    } else {
                        false
                    }
                });

                let final_content = if !starts_with_toplevel && !content.trim().is_empty() {
                    // Check if we have a name to wrap?
                    if let Some(n) = &item_ident {
                        // Indent content for auto-wrapping
                        let indented = content
                            .lines()
                            .map(|l| format!("      {}", l))
                            .collect::<Vec<_>>()
                            .join("\n");
                        format!("components:\n  schemas:\n    {}:\n{}", n, indented)
                    } else {
                        content.clone()
                    }
                } else {
                    content.clone()
                };

                self.items.push(ExtractedItem::Schema {
                    name: item_ident.clone(),
                    content: final_content,
                });
            }
        }
    }
}

impl<'ast> Visit<'ast> for OpenApiVisitor {
    fn visit_file(&mut self, i: &'ast File) {
        self.check_attributes(&i.attrs, None);
        visit::visit_file(self, i);
    }

    fn visit_item_fn(&mut self, i: &'ast ItemFn) {
        self.check_attributes(&i.attrs, None);
        visit::visit_item_fn(self, i);
    }

    fn visit_item_struct(&mut self, i: &'ast ItemStruct) {
        let ident = i.ident.to_string();
        self.check_attributes(&i.attrs, Some(ident));
        visit::visit_item_struct(self, i);
    }

    fn visit_item_enum(&mut self, i: &'ast ItemEnum) {
        let ident = i.ident.to_string();
        self.check_attributes(&i.attrs, Some(ident));
        visit::visit_item_enum(self, i);
    }

    fn visit_item_mod(&mut self, i: &'ast ItemMod) {
        self.check_attributes(&i.attrs, None);
        visit::visit_item_mod(self, i);
    }

    fn visit_impl_item_fn(&mut self, i: &'ast ImplItemFn) {
        self.check_attributes(&i.attrs, None);
        visit::visit_impl_item_fn(self, i);
    }
}

pub fn extract_from_file(path: std::path::PathBuf) -> crate::error::Result<Vec<ExtractedItem>> {
    let content = std::fs::read_to_string(&path)?;
    let parsed_file = syn::parse_file(&content).map_err(|e| crate::error::Error::Parse {
        file: path,
        source: e,
    })?;

    let mut visitor = OpenApiVisitor::default();
    visitor.visit_file(&parsed_file);

    Ok(visitor.items)
}
