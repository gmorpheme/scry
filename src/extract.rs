//! The main extraction machinery including ContentIterator for
//! iterating over lines / paragraphs in a ScrivenerProject.
use crate::annot;
use crate::bundle::BinderItemFolder;
use crate::bundle::Bundle;
use crate::rtf;
use crate::scrivx::{BinderItem, BinderItemType, BinderIterator, ScrivenerProject};
use crate::tag;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, BufRead};
use uuid::Uuid;

/// Specifies folders to extract
#[derive(PartialEq, Eq, Hash)]
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

/// Extracts content from scrivener project
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

    /// Create a binder iterator using configured folder specs
    fn binder_iterator(&self) -> BinderIterator {
        let roots: Vec<_> = self
            .project
            .binder
            .binder_items
            .iter()
            .filter(|it| self.folder_specs.iter().any(|spec| matches(it, spec)))
            .collect();

        BinderIterator::new(roots)
    }

    /// Return an iterator over all selected content
    pub fn iter(&self) -> ExtractionIterator {
        ExtractionIterator::new(&self.bundle, self.binder_iterator(), &self.content_specs)
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
