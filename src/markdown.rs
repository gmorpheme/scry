//! Markdown output mode for scry
//!
//! Emits structured Markdown files preserving the Scrivener binder
//! hierarchy as heading levels. Supports single-file (one .md per
//! top-level folder) and split (one .md per binder item) modes.

use crate::annot;
use crate::bundle::Bundle;
use crate::error::Result;
use crate::extract::{ContentSpec, FolderSpec};
use crate::rtf;
use crate::scrivx::{BinderItem, BinderItemType, ScrivenerProject};
use crate::tag;

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

/// Sanitise a title into a filename-safe slug
fn slugify(title: &str) -> String {
    let s: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    // collapse runs of hyphens and trim
    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in s.chars() {
        if c == '-' {
            if !prev_hyphen && !result.is_empty() {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    result.trim_end_matches('-').to_string()
}

/// Generate a heading line at the given depth (0-based)
fn heading(depth: usize, title: &str) -> String {
    let level = (depth + 1).min(6);
    let hashes: String = "#".repeat(level);
    format!("{} {}", hashes, title)
}

/// Format a synopsis as an HTML comment
fn synopsis_comment(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.contains('\n') {
        format!("<!-- Synopsis:\n{}\n-->", trimmed)
    } else {
        format!("<!-- Synopsis: {} -->", trimmed)
    }
}

/// Format a note line as an HTML comment
fn note_comment(text: &str) -> String {
    format!("<!-- Note: {} -->", text.trim())
}

/// Format an inline annotation as an HTML comment
fn inline_comment(text: &str) -> String {
    format!("<!-- Inline: {} -->", text.trim())
}

/// Format a non-text item placeholder
fn asset_placeholder(item: &BinderItem, bundle: &Bundle, include_path: bool) -> String {
    let type_label = match item.r#type {
        BinderItemType::PDF => "PDF",
        BinderItemType::Image => "Image",
        BinderItemType::WebArchive => "WebArchive",
        _ => "Other",
    };

    if include_path {
        let folder = bundle.binder_item_folder(&item.uuid);
        format!(
            "<!-- [{}: {} ({})] -->",
            type_label,
            item.title,
            folder.display()
        )
    } else {
        format!("<!-- [{}: {}] -->", type_label, item.title)
    }
}

/// Whether a binder item type contains text content
fn is_text_type(t: &BinderItemType) -> bool {
    matches!(
        t,
        BinderItemType::Text
            | BinderItemType::Folder
            | BinderItemType::DraftFolder
            | BinderItemType::ResearchFolder
            | BinderItemType::TrashFolder
    )
}

/// Collects markdown lines for a single binder item
fn emit_item(
    item: &BinderItem,
    depth: usize,
    bundle: &Bundle,
    content_specs: &HashSet<ContentSpec>,
    asset_path: bool,
) -> Vec<String> {
    let mut lines = Vec::new();

    // Heading
    lines.push(heading(depth, &item.title));
    lines.push(String::new());

    // Non-text items get a placeholder only
    if !is_text_type(&item.r#type) {
        lines.push(asset_placeholder(item, bundle, asset_path));
        lines.push(String::new());
        return lines;
    }

    let folder = bundle.binder_item_content(&item.uuid);

    // Synopsis
    if content_specs.contains(&ContentSpec::Synopsis) {
        if let Some(path) = folder.synopsis() {
            if let Ok(file) = File::open(path) {
                let mut content = String::new();
                if io::BufReader::new(file)
                    .read_to_string(&mut content)
                    .is_ok()
                    && !content.trim().is_empty()
                {
                    lines.push(synopsis_comment(&content));
                    lines.push(String::new());
                }
            }
        }
    }

    // Content
    if content_specs.contains(&ContentSpec::Content) {
        if let Some(path) = folder.content() {
            if path.extension() == Some(OsStr::new("rtf")) {
                if let Ok(paragraphs) = rtf::parse_rtf_file(path) {
                    let text_lines: Vec<String> = annot::skip_annotations(paragraphs)
                        .map(tag::strip_tags)
                        .filter(|s| !s.is_empty())
                        .collect();
                    for line in text_lines {
                        lines.push(line);
                        lines.push(String::new());
                    }
                }
            }
        }
    }

    // Notes
    if content_specs.contains(&ContentSpec::Notes) {
        if let Some(path) = folder.notes() {
            if path.extension() == Some(OsStr::new("rtf")) {
                if let Ok(paragraphs) = rtf::parse_rtf_file(path) {
                    let note_lines: Vec<String> = paragraphs.filter(|s| !s.is_empty()).collect();
                    let has_notes = !note_lines.is_empty();
                    for line in note_lines {
                        lines.push(note_comment(&line));
                    }
                    if has_notes {
                        lines.push(String::new());
                    }
                }
            }
        }
    }

    // Inline annotations
    if content_specs.contains(&ContentSpec::Inlines) {
        if let Some(path) = folder.content() {
            if path.extension() == Some(OsStr::new("rtf")) {
                if let Ok(paragraphs) = rtf::parse_rtf_file(path) {
                    let annot_lines: Vec<String> = annot::only_annotations(paragraphs)
                        .filter(|s| !s.is_empty())
                        .collect();
                    for line in &annot_lines {
                        lines.push(inline_comment(line));
                    }
                    if !annot_lines.is_empty() {
                        lines.push(String::new());
                    }
                }
            }
        }
    }

    lines
}

/// Emit markdown in single-file mode: one .md per top-level folder
pub fn emit_single_file(
    project: &ScrivenerProject,
    bundle: &Bundle,
    output_dir: &Path,
    project_name: &str,
    folder_specs: &HashSet<FolderSpec>,
    content_specs: &HashSet<ContentSpec>,
    asset_path: bool,
) -> Result<()> {
    let project_dir = output_dir.join(project_name);
    fs::create_dir_all(&project_dir)?;

    // Group items by top-level folder
    let top_level: Vec<&BinderItem> = project
        .binder
        .binder_items
        .iter()
        .filter(|it| folder_specs.iter().any(|spec| matches_folder(it, spec)))
        .collect();

    for root in top_level {
        let filename = format!("{}.md", slugify(&root.title));
        let filepath = project_dir.join(&filename);

        let mut all_lines: Vec<String> = Vec::new();

        // Walk the tree depth-first
        walk_item(root, 0, bundle, content_specs, asset_path, &mut all_lines);

        // Write file
        let mut file = File::create(&filepath)?;
        let content = all_lines.join("\n");
        file.write_all(content.as_bytes())?;

        eprintln!("Wrote {}", filepath.display());
    }

    Ok(())
}

/// Emit markdown in split mode: one .md per leaf item, directories for folders
pub fn emit_split(
    project: &ScrivenerProject,
    bundle: &Bundle,
    output_dir: &Path,
    project_name: &str,
    folder_specs: &HashSet<FolderSpec>,
    content_specs: &HashSet<ContentSpec>,
    asset_path: bool,
) -> Result<()> {
    let project_dir = output_dir.join(project_name);

    let top_level: Vec<&BinderItem> = project
        .binder
        .binder_items
        .iter()
        .filter(|it| folder_specs.iter().any(|spec| matches_folder(it, spec)))
        .collect();

    for root in top_level {
        let root_dir = project_dir.join(slugify(&root.title));
        write_split_item(root, &root_dir, 0, bundle, content_specs, asset_path)?;
    }

    Ok(())
}

/// Recursively write split-mode files
fn write_split_item(
    item: &BinderItem,
    parent_dir: &Path,
    index: usize,
    bundle: &Bundle,
    content_specs: &HashSet<ContentSpec>,
    asset_path: bool,
) -> Result<()> {
    let numbered_slug = format!("{:02}-{}", index + 1, slugify(&item.title));

    if item.children.binder_items.is_empty() {
        // Leaf item: write a file
        fs::create_dir_all(parent_dir)?;
        let filepath = parent_dir.join(format!("{}.md", numbered_slug));

        let mut lines: Vec<String> = Vec::new();
        // In split mode, heading levels reset — this item is depth 0
        let item_lines = emit_item(item, 0, bundle, content_specs, asset_path);
        lines.extend(item_lines);

        let mut file = File::create(&filepath)?;
        file.write_all(lines.join("\n").as_bytes())?;

        eprintln!("Wrote {}", filepath.display());
    } else {
        // Folder: create directory, write children
        let folder_dir = parent_dir.join(&numbered_slug);
        fs::create_dir_all(&folder_dir)?;

        // If the folder itself has content, write an index file
        let folder_content = bundle.binder_item_content(&item.uuid);
        let has_own_content = folder_content.content().is_some()
            || folder_content.synopsis().is_some()
            || folder_content.notes().is_some();

        if has_own_content {
            let index_path = folder_dir.join("_index.md");
            let lines = emit_item(item, 0, bundle, content_specs, asset_path);
            let mut file = File::create(&index_path)?;
            file.write_all(lines.join("\n").as_bytes())?;
        }

        for (i, child) in item.children.binder_items.iter().enumerate() {
            write_split_item(child, &folder_dir, i, bundle, content_specs, asset_path)?;
        }
    }

    Ok(())
}

/// Walk a binder item and its children, collecting markdown lines
fn walk_item(
    item: &BinderItem,
    depth: usize,
    bundle: &Bundle,
    content_specs: &HashSet<ContentSpec>,
    asset_path: bool,
    lines: &mut Vec<String>,
) {
    let item_lines = emit_item(item, depth, bundle, content_specs, asset_path);
    lines.extend(item_lines);

    for child in &item.children.binder_items {
        walk_item(child, depth + 1, bundle, content_specs, asset_path, lines);
    }
}

/// Check if an item matches a folder spec (mirrors extract.rs logic)
fn matches_folder(item: &BinderItem, folder_spec: &FolderSpec) -> bool {
    match folder_spec {
        FolderSpec::DraftFolder => item.r#type == BinderItemType::DraftFolder,
        FolderSpec::ResearchFolder => item.r#type == BinderItemType::ResearchFolder,
        FolderSpec::TrashFolder => item.r#type == BinderItemType::TrashFolder,
        FolderSpec::NamedFolder(ref s) => &item.title == s,
        FolderSpec::Any => item.r#type != BinderItemType::TrashFolder,
    }
}
