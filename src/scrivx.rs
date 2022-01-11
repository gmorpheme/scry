///! Representation and parsing of .scrivx project files
use quick_xml::de::{from_reader, DeError};
use serde::{Deserialize, Deserializer};
use std::io::{BufReader, Read};
use uuid::Uuid;

/// Top level project element
#[derive(Debug, Deserialize, PartialEq)]
pub struct ScrivenerProject {
    #[serde(rename = "Identifier", default)]
    pub identifier: Uuid,
    #[serde(rename = "Version", default)]
    pub version: String,
    #[serde(rename = "Creator", default)]
    pub creator: String,
    #[serde(rename = "Device", default)]
    pub device: String,
    #[serde(rename = "Author", default)]
    pub author: String,
    #[serde(rename = "Binder")]
    pub binder: Binder,
    #[serde(rename = "ModID")]
    pub mod_id: Uuid,
}

impl ScrivenerProject {
    /// Parse a scrivx project file
    pub fn parse<T: Read>(input: T) -> Result<Self, DeError> {
        let r = BufReader::new(input);
        from_reader(r)
    }

    /// An iterator over all items in the project's binder
    pub fn iter(&self) -> BinderIterator {
        BinderIterator::new(self.binder.binder_items.iter().collect())
    }

    /// Find the draft folder
    pub fn draft(&self) -> &BinderItem {
        for i in self.iter() {
            if i.r#type == BinderItemType::DraftFolder {
                return i;
            }
        }
        panic!("No draft folder in project!")
    }
}

/// Binder item types
#[derive(Debug, Deserialize, PartialEq)]
pub enum BinderItemType {
    /// The single draft folder
    DraftFolder,
    /// The research folder
    ResearchFolder,
    /// The trash folder
    TrashFolder,
    /// A binder folder
    Folder,
    /// A normal text item
    Text,
    /// A PDF
    PDF,
    /// An Image
    Image,
    /// Archived web content
    WebArchive,
    /// Other content type
    Other,
}

impl Default for BinderItemType {
    fn default() -> Self {
        BinderItemType::Other
    }
}

/// Deserialise a boolean from Yes / No
fn de_from_yes_no<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(s == "Yes")
}

/// Binder item metadata
#[derive(Debug, Deserialize, PartialEq)]
pub struct BinderItemMetadata {
    #[serde(rename = "LabelID", default)]
    pub label_id: i32,
    #[serde(rename = "StatusID", default)]
    pub status_id: i32,
    #[serde(
        rename = "IncludeInCompile",
        deserialize_with = "de_from_yes_no",
        default
    )]
    pub include_in_compile: bool,
}

/// A binder item
///
/// Maybe folder, text or other content
#[derive(Debug, Deserialize, PartialEq)]
pub struct BinderItem {
    #[serde(rename = "UUID", default)]
    pub uuid: Uuid,
    #[serde(rename = "Type", default)]
    pub r#type: BinderItemType,
    #[serde(rename = "Title", default)]
    pub title: String,
    #[serde(rename = "Children", default)]
    pub children: Children,
}

impl BinderItem {
    /// Iterate over this item and its descendents
    pub fn iter(&self) -> BinderIterator {
        BinderIterator::new_from_root(self)
    }
}

/// The binder section of a project
#[derive(Debug, Deserialize, PartialEq)]
pub struct Binder {
    #[serde(rename = "BinderItem")]
    pub binder_items: Vec<BinderItem>,
}

impl Binder {
    /// An iterator over all items in the binder
    pub fn iter(&self) -> BinderIterator {
        BinderIterator::new(self.binder_items.iter().collect())
    }
}

#[derive(Debug, Deserialize, PartialEq, Default)]
pub struct Children {
    #[serde(rename = "BinderItem")]
    pub binder_items: Vec<BinderItem>,
}

/// An iterator over binder items
pub struct BinderIterator<'a> {
    stack: Vec<&'a BinderItem>,
}

impl<'a> BinderIterator<'a> {
    /// An iterator over the binder's items
    pub fn new(roots: Vec<&'a BinderItem>) -> BinderIterator<'a> {
        BinderIterator {
            stack: roots.into_iter().rev().collect(),
        }
    }

    pub fn new_from_root(root: &'a BinderItem) -> BinderIterator<'a> {
        BinderIterator { stack: vec![root] }
    }
}

impl<'a> Iterator for BinderIterator<'a> {
    type Item = &'a BinderItem;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.stack.pop() {
            if !item.children.binder_items.is_empty() {
                self.stack.extend(item.children.binder_items.iter().rev());
                Some(item)
            } else {
                Some(item)
            }
        } else {
            None
        }
    }
}
