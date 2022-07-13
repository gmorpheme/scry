//! A set of extraction machinery including
//!
//! - ContentIterator for iterating over lines / paragraphs
//! - Extractor for extracting textual content from a project
//! - JsonItemiser for outputing item data as JSON
//!

use crate::annot;
use crate::bundle::BinderItemFolder;
use crate::bundle::Bundle;
use crate::error::ScryError;
use crate::rtf;
use crate::scrivx::{BinderItem, BinderItemType, BinderIterator, ScrivenerProject};
use crate::tag;
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs::File,
    io::{self, stdout, BufRead, Read},
};
use uuid::Uuid;

use json::JsonValue;

/// Specifies folders to extract
#[derive(PartialEq, Eq, Hash, Clone)]
pub enum FolderSpec {
    /// The draft folder
    DraftFolder,
    /// The research folder
    ResearchFolder,
    /// The trash folder
    TrashFolder,
    /// A named top-level folder
    NamedFolder(String),
    /// Any top-level folder except trash (will match conflicts)
    Any,
}

/// Returns true if item matches the folder spec
fn matches(item: &BinderItem, folder_spec: &FolderSpec) -> bool {
    match folder_spec {
        FolderSpec::DraftFolder => item.r#type == BinderItemType::DraftFolder,
        FolderSpec::ResearchFolder => item.r#type == BinderItemType::ResearchFolder,
        FolderSpec::TrashFolder => item.r#type == BinderItemType::TrashFolder,
        FolderSpec::NamedFolder(ref s) => &item.title == s,
        FolderSpec::Any => item.r#type != BinderItemType::TrashFolder,
    }
}

/// Create a binder iterator from a project and set of folder specifications
pub fn binder_iterator(
    project: &ScrivenerProject,
    folder_specs: HashSet<FolderSpec>,
) -> BinderIterator {
    let roots: Vec<_> = project
        .binder
        .binder_items
        .iter()
        .filter(|it| folder_specs.iter().any(|spec| matches(it, spec)))
        .collect();

    BinderIterator::new(roots)
}

/// Specifies content type to extract for each item
#[derive(PartialEq, Eq, Hash, Clone)]
pub enum ContentSpec {
    /// Item title
    Title,
    /// Lines from item synopsis
    Synopsis,
    /// Paragraphs from item RTF content
    Content,
    /// Paragraphs from item notes
    Notes,
    /// Inline comments from item RTF content
    Inlines,
    /// Out of line comments from item
    Comments,
}

/// Iterator over selected content in a Scrivener binder item
pub struct ContentIterator {
    /// Item UUID
    _uuid: Uuid,
    /// Item title
    title: String,
    /// Item folder
    folder: BinderItemFolder,
    /// Content specs remaining to satisfy
    content_specs: HashSet<ContentSpec>,
    /// Current iterator
    iterator: Option<Box<dyn Iterator<Item = String>>>,
}

impl ContentIterator {
    /// Construct a new content iterator for an item
    pub fn new(
        uuid: Uuid,
        title: String,
        folder_content: BinderItemFolder,
        content_specs: &HashSet<ContentSpec>,
    ) -> Self {
        ContentIterator {
            _uuid: uuid,
            title,
            folder: folder_content,
            content_specs: content_specs.clone(),
            iterator: None,
        }
    }

    /// Create iterator over synopsis lines
    fn synopsis_line_iterator(&self) -> Option<io::Lines<io::BufReader<File>>> {
        if let Some(path) = self.folder.synopsis() {
            let file = File::open(path).ok()?;
            Some(io::BufReader::new(file).lines())
        } else {
            None
        }
    }

    /// Create iterator over content paragraphs
    fn content_paragraph_iterator(
        &self,
    ) -> Option<annot::AnnotationAdapter<rtf::ParagraphIterator>> {
        if let Some(path) = self.folder.content() {
            if path.extension() == Some(OsStr::new("rtf")) {
                rtf::parse_rtf_file(path).ok().map(annot::skip_annotations)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Create iterator over inline annotations
    fn content_annotation_iterator(
        &self,
    ) -> Option<annot::AnnotationAdapter<rtf::ParagraphIterator>> {
        if let Some(path) = self.folder.content() {
            if path.extension() == Some(OsStr::new("rtf")) {
                rtf::parse_rtf_file(path).ok().map(annot::only_annotations)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Create iterator over notes paragraphs
    fn notes_paragraph_iterator(&self) -> Option<rtf::ParagraphIterator> {
        if let Some(path) = self.folder.notes() {
            if path.extension() == Some(OsStr::new("rtf")) {
                rtf::parse_rtf_file(path).ok()
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Load up the next iterator based on the remaining content specs
    fn load_iterator(&mut self) -> bool {
        if self.content_specs.remove(&ContentSpec::Title) {
            self.iterator = Some(Box::new(std::iter::once(self.title.clone())));
            return true;
        }

        if self.content_specs.remove(&ContentSpec::Synopsis) {
            if let Some(it) = self.synopsis_line_iterator() {
                self.iterator = Some(Box::new(it.map(|s| s.unwrap()))); // TODO: it of result
                return true;
            }
        }

        if self.content_specs.remove(&ContentSpec::Content) {
            if let Some(it) = self.content_paragraph_iterator() {
                self.iterator = Some(Box::new(it.map(tag::strip_tags)));
                return true;
            }
        }

        if self.content_specs.remove(&ContentSpec::Notes) {
            if let Some(it) = self.notes_paragraph_iterator() {
                self.iterator = Some(Box::new(it));
                return true;
            }
        }

        if self.content_specs.remove(&ContentSpec::Inlines) {
            if let Some(it) = self.content_annotation_iterator() {
                self.iterator = Some(Box::new(it));
                return true;
            }
        }

        if self.content_specs.remove(&ContentSpec::Comments) {
            // TODO: comment iterator
        }

        false
    }
}

impl Iterator for ContentIterator {
    type Item = String;

    /// Next string in selected item content
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(ref mut it) = self.iterator {
            if let Some(text) = it.next() {
                Some(text)
            } else if self.load_iterator() {
                self.next()
            } else {
                None
            }
        } else if self.load_iterator() {
            self.next()
        } else {
            None
        }
    }
}

/// Extracts pure textual content from Scrivener Project
///
/// All structure is eradicted and the output is a flat list of
/// strings. The content types and folders that are included are
/// specified. RTF data is split into paragraphs, plain text into
/// lines. No UUIDs or non-textual data is included.
pub struct Extractor {
    /// The Scrivener project file
    project: ScrivenerProject,
    /// The bundle folder containing content
    bundle: Bundle,
    /// Top-level folders to include
    folder_specs: HashSet<FolderSpec>,
    /// Content type to include
    content_specs: HashSet<ContentSpec>,
}

impl Extractor {
    /// Construct a new Extractor for a scrivener project
    pub fn new(
        project: ScrivenerProject,
        bundle: Bundle,
        folder_specs: HashSet<FolderSpec>,
        content_specs: HashSet<ContentSpec>,
    ) -> Self {
        Extractor {
            project,
            bundle,
            folder_specs,
            content_specs,
        }
    }

    /// Return an iterator over all selected content
    pub fn iter(&self) -> ExtractionIterator {
        ExtractionIterator::new(
            &self.bundle,
            binder_iterator(&self.project, self.folder_specs.clone()),
            &self.content_specs,
        )
    }
}

/// An iterator over all the selected content in the binder
pub struct ExtractionIterator<'a> {
    /// Bundle for locating content
    bundle: &'a Bundle,
    /// Where we're up to in the binder
    binder_iterator: BinderIterator<'a>,
    /// Where we're up to in the current item
    content_iterator: Option<ContentIterator>,
    /// Content to include
    content_specs: &'a HashSet<ContentSpec>,
}

impl<'a> ExtractionIterator<'a> {
    /// Create a new extraction iterator using extractor's settings
    pub fn new(
        bundle: &'a Bundle,
        binder_iterator: BinderIterator<'a>,
        content_specs: &'a HashSet<ContentSpec>,
    ) -> Self {
        ExtractionIterator {
            bundle,
            binder_iterator,
            content_iterator: None,
            content_specs,
        }
    }

    /// Load up the next content iterator
    fn load_content_iterator(&mut self) -> bool {
        if let Some(item) = self.binder_iterator.next() {
            self.content_iterator = Some(ContentIterator::new(
                item.uuid,
                item.title.clone(),
                self.bundle.binder_item_content(&item.uuid),
                self.content_specs,
            ));
            true
        } else {
            false
        }
    }
}

impl<'a> Iterator for ExtractionIterator<'a> {
    type Item = String;

    /// Get next item from content iterator unless it is exhausted in
    /// which case load up a content iterator for the next item
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(ref mut it) = self.content_iterator {
            if let Some(text) = it.next() {
                Some(text)
            } else if self.load_content_iterator() {
                self.next()
            } else {
                None
            }
        } else if self.load_content_iterator() {
            self.next()
        } else {
            None
        }
    }
}

/// Outputs flat list of structured items to stdout as JSON.
///
/// Internal item structure is preserved but binder structure is
/// collapsed into a depth first listing.
///
/// (no need to abstract, no need to stream for now)
pub struct JsonItemiser {
    /// content items to include in JSON
    content_specs: HashSet<ContentSpec>,
    /// items accumulated so far
    items: Vec<JsonValue>,
}

impl JsonItemiser {
    /// Create a new itemiser to output the content types specified
    pub fn new(content_specs: HashSet<ContentSpec>) -> Self {
        JsonItemiser {
            items: vec![],
            content_specs,
        }
    }

    /// Accept a binder item and massage into JSON object
    pub fn consume_item(
        &mut self,
        item: &BinderItem,
        folder: &BinderItemFolder,
    ) -> Result<(), ScryError> {
        let mut object = JsonValue::new_object();
        // x-scrivener-item links need uppercase GUIDS - might as well
        // ensure it here:
        object.insert("uuid", item.uuid.to_string().to_ascii_uppercase())?;
        object.insert("type", item.r#type.to_string())?;

        if self.content_specs.contains(&ContentSpec::Title) {
            object.insert("title", item.title.clone())?;
        }

        if self.content_specs.contains(&ContentSpec::Synopsis) {
            if let Some(path) = folder.synopsis() {
                let file = File::open(path)?;
                let mut content = String::new();
                io::BufReader::new(file).read_to_string(&mut content)?;
                object.insert("synopsis", content)?;
            }
        }

        if self.content_specs.contains(&ContentSpec::Content) {
            if let Some(path) = folder.content() {
                if path.extension() == Some(OsStr::new("rtf")) {
                    let content: Vec<String> =
                        annot::skip_annotations(rtf::parse_rtf_file(path)?).collect();
                    object.insert("content", content)?;
                }
            }
        }

        if self.content_specs.contains(&ContentSpec::Inlines) {
            if let Some(path) = folder.content() {
                if path.extension() == Some(OsStr::new("rtf")) {
                    let content: Vec<String> =
                        annot::only_annotations(rtf::parse_rtf_file(path)?).collect();
                    object.insert("inlines", content)?;
                }
            }
        }

        if self.content_specs.contains(&ContentSpec::Notes) {
            if let Some(path) = folder.notes() {
                if path.extension() == Some(OsStr::new("rtf")) {
                    let content: Vec<String> = rtf::parse_rtf_file(path)?.collect();
                    object.insert("notes", content)?;
                }
            }
        }

        if self.content_specs.contains(&ContentSpec::Comments) {
            // TODO: comments
        }

        self.items.push(object);
        Ok(())
    }

    /// Wrap in an { "items": [...] } object and dump to stdout
    pub fn write_to_stdout(self) -> Result<(), ScryError> {
        let mut wrapper = JsonValue::new_object();
        wrapper.insert("items", self.items)?;
        wrapper.write(&mut stdout())?;
        Ok(())
    }
}
