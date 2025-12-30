use std::collections::HashSet;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, File, ImplItemFn, ItemEnum, ItemFn, ItemMod, ItemStruct};

/// Visitor that traverses the Rust AST to find `#[doc]` attributes containing OpenAPI definitions.
#[derive(Default)]
pub struct OpenApiVisitor {
    pub snippets: Vec<String>,
    pub defined_schemas: HashSet<String>,
}

impl OpenApiVisitor {
    /// Helper to check attributes on an item.
    /// If `item_ident` is provided, it records it as a defined schema if an OpenAPI comment is found.
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

        // Unindent logic: find strictly common indent
        let min_indent = doc_lines
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.chars().take_while(|c| *c == ' ').count())
            .min()
            .unwrap_or(0);

        let unindented: Vec<String> = doc_lines
            .iter()
            .map(|line| {
                if line.len() >= min_indent {
                    line[min_indent..].to_string()
                } else {
                    line.clone()
                }
            })
            .collect();

        // Join lines with newline to reconstruct the block
        let full_doc = unindented.join("\n");
        let trimmed = full_doc.trim();

        let mut found_openapi = false;

        // Check for @openapi OR JSON start
        if trimmed.starts_with("@openapi") {
            let content = trimmed.strip_prefix("@openapi").unwrap_or("").trim();
            if !content.is_empty() {
                self.snippets.push(content.to_string());
                found_openapi = true;
            }
        } else if trimmed.starts_with('{') {
            // Heuristic for JSON blocks
            self.snippets.push(trimmed.to_string());
            found_openapi = true;
        }

        // Functionality: Register schema name if we found an OpenAPI block on a struct/enum
        if found_openapi {
            if let Some(ident) = item_ident {
                self.defined_schemas.insert(ident);
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
        self.check_attributes(&i.attrs, None); // Functions usually contain paths, not schemas
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

pub struct Extracted {
    pub snippets: Vec<String>,
    pub schemas: HashSet<String>,
}

pub fn extract_from_file(path: std::path::PathBuf) -> crate::error::Result<Extracted> {
    let content = std::fs::read_to_string(&path)?;
    let parsed_file = syn::parse_file(&content).map_err(|e| crate::error::Error::Parse {
        file: path,
        source: e,
    })?;

    let mut visitor = OpenApiVisitor::default();
    visitor.visit_file(&parsed_file);

    Ok(Extracted {
        snippets: visitor.snippets,
        schemas: visitor.defined_schemas,
    })
}
