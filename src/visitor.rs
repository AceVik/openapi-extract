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

        if doc_lines.is_empty() {
            return;
        }

        // If we have generated content, append it.
        // We assume the user placed `@openapi` marker in the doc comments.
        // If we just append it, it becomes part of the body.
        if let Some(generated) = generated_content {
            doc_lines.push(generated);
        }

        let full_doc = doc_lines.join("\n");
        // We pass the item's start line as the base line number for the snippet.
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
                // Auto-Wrap Heuristic
                // If content does NOT start with top-level keys, wrap it.
                // Keys: openapi, info, paths, components, tags, servers, security
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
                    line,
                });
            }
        }
    }
}

impl<'ast> Visit<'ast> for OpenApiVisitor {
    fn visit_file(&mut self, i: &'ast File) {
        // File-level docs usually at top
        self.check_attributes(&i.attrs, None, 1, None);
        visit::visit_file(self, i);
    }

    fn visit_item_fn(&mut self, i: &'ast ItemFn) {
        self.check_attributes(&i.attrs, None, i.span().start().line, None);
        visit::visit_item_fn(self, i);
    }

    fn visit_item_struct(&mut self, i: &'ast ItemStruct) {
        let ident = i.ident.to_string();
        self.check_attributes(&i.attrs, Some(ident), i.span().start().line, None);
        visit::visit_item_struct(self, i);
    }

    fn visit_item_enum(&mut self, i: &'ast ItemEnum) {
        let ident = i.ident.to_string();

        // Auto-Enum Extraction
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
        self.check_attributes(&i.attrs, None, i.span().start().line, None);
        visit::visit_item_mod(self, i);
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
