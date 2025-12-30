use crate::scanner::substitute_variables;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, File, ImplItemFn, ItemEnum, ItemFn, ItemMod, ItemStruct};

/// Visitor that traverses the Rust AST to find `#[doc]` attributes containing OpenAPI definitions.
#[derive(Default)]
pub struct OpenApiVisitor {
    pub snippets: Vec<String>,
}

impl OpenApiVisitor {
    fn check_attributes(&mut self, attrs: &[Attribute]) {
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

        // Check for @openapi OR JSON start
        if trimmed.starts_with("@openapi") {
            let content = trimmed.strip_prefix("@openapi").unwrap_or("").trim();
            if !content.is_empty() {
                self.snippets.push(substitute_variables(content));
            }
        } else if trimmed.starts_with('{') {
            // Heuristic: if it looks like JSON and is inside a doc comment,
            // users might expect it to be picked up if they explicitly want to support it
            // BUT, standard behavior is @openapi.
            // The prompt said: "Comments containing JSON (detect if block starts with `{`)."
            // I will support it but only if it parses as valid JSON? No, just extract.
            // Actually, a block starting with `{` in a doc comment could be code example.
            // To be safe, I'll strictly follow the prompt but it's risky.
            // "Extact content only if the comment starts with @openapi" was the V1 rule.
            // V2 Prompt: "detect if block starts with {"
            // Ok, I will add it.
            self.snippets.push(substitute_variables(trimmed));
        }
    }
}

impl<'ast> Visit<'ast> for OpenApiVisitor {
    fn visit_file(&mut self, i: &'ast File) {
        self.check_attributes(&i.attrs);
        visit::visit_file(self, i);
    }

    fn visit_item_fn(&mut self, i: &'ast ItemFn) {
        self.check_attributes(&i.attrs);
        visit::visit_item_fn(self, i);
    }

    fn visit_item_struct(&mut self, i: &'ast ItemStruct) {
        self.check_attributes(&i.attrs);
        visit::visit_item_struct(self, i);
    }

    fn visit_item_enum(&mut self, i: &'ast ItemEnum) {
        self.check_attributes(&i.attrs);
        visit::visit_item_enum(self, i);
    }

    fn visit_item_mod(&mut self, i: &'ast ItemMod) {
        self.check_attributes(&i.attrs);
        visit::visit_item_mod(self, i);
    }

    fn visit_impl_item_fn(&mut self, i: &'ast ImplItemFn) {
        self.check_attributes(&i.attrs);
        visit::visit_impl_item_fn(self, i);
    }
}

pub fn extract_from_file(path: std::path::PathBuf) -> crate::error::Result<Vec<String>> {
    let content = std::fs::read_to_string(&path)?;
    let parsed_file = syn::parse_file(&content).map_err(|e| crate::error::Error::Parse {
        file: path,
        source: e,
    })?;

    let mut visitor = OpenApiVisitor::default();
    visitor.visit_file(&parsed_file);

    Ok(visitor.snippets)
}
