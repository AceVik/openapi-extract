use crate::error::{Error, Result};
use crate::visitor;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Replaces {{CARGO_PKG_VERSION}} with the environment variable or default.
pub fn substitute_variables(content: &str) -> String {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    content.replace("{{CARGO_PKG_VERSION}}", &version)
}

/// Scans directories for .rs files and other allowed types.
/// Returns a list of strings (OpenAPI snippets or full file contents).
pub fn scan_directories(roots: &[PathBuf], includes: &[PathBuf]) -> Result<Vec<String>> {
    let mut all_snippets = Vec::new();
    let mut files_found = false;

    // 1. Process directories
    for root in roots {
        for entry in WalkDir::new(root) {
            let entry = entry.map_err(|e| Error::Io(std::io::Error::other(e)))?;
            let path = entry.path();

            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    match ext {
                        "rs" => {
                            files_found = true;
                            let snippets = visitor::extract_from_file(path.to_path_buf())?;
                            all_snippets.extend(snippets);
                        }
                        "json" | "yaml" | "yml" => {
                            files_found = true;
                            let content = std::fs::read_to_string(path)?;
                            let substituted = substitute_variables(&content);
                            all_snippets.push(substituted);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // 2. Process explicit includes
    for path in includes {
        if path.exists() {
            files_found = true;
            // If it's a rust file, extract; otherwise treat as raw content
            if path.extension().is_some_and(|ext| ext == "rs") {
                let snippets = visitor::extract_from_file(path.to_path_buf())?;
                all_snippets.extend(snippets);
            } else {
                let content = std::fs::read_to_string(path)?;
                let substituted = substitute_variables(&content);
                all_snippets.push(substituted);
            }
        }
    }

    if !files_found && !roots.is_empty() && !includes.is_empty() {
        return Err(Error::NoFilesFound);
    }

    Ok(all_snippets)
}
