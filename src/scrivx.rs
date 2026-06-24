//! Representation and parsing of .scrivx project files
use quick_xml::de::{from_reader, DeError};
use serde::{Deserialize, Deserializer};
use std::{
    collections::VecDeque,
    fmt,
    io::{BufReader, Read},
};
use uuid::Uuid;

/// Top level project element
#[derive(Debug, Deserialize, PartialEq)]
pub struct ScrivenerProject {
    #[serde(rename = "@Identifier", default)]
    pub identifier: Uuid,
    #[serde(rename = "@Version", default)]
    pub version: String,
    #[serde(rename = "@Creator", default)]
    pub creator: String,
    #[serde(rename = "@Device", default)]
    pub device: String,
    #[serde(rename = "@Author", default)]
    pub author: String,
    #[serde(rename = "@Modified", default)]
    pub modified: String,
    #[serde(rename = "@ModID", default)]
    pub mod_id: Uuid,
    #[serde(rename = "Binder")]
    pub binder: Binder,
    #[serde(rename = "Collections")]
    pub collections: Collections,
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
            if i.item.r#type == BinderItemType::DraftFolder {
                return i.item;
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

impl fmt::Display for BinderItemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            BinderItemType::DraftFolder => write!(f, "DraftFolder"),
            BinderItemType::ResearchFolder => write!(f, "ResearchFolder"),
            BinderItemType::TrashFolder => write!(f, "TrashFolder"),
            BinderItemType::Folder => write!(f, "Folder"),
            BinderItemType::Text => write!(f, "Text"),
            BinderItemType::PDF => write!(f, "PDF"),
            BinderItemType::Image => write!(f, "Image"),
            BinderItemType::WebArchive => write!(f, "WebArchive"),
            BinderItemType::Other => write!(f, "Other"),
        }
    }
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
    #[serde(rename = "UUID", default)]
    pub section_type: Uuid,
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
    #[serde(rename = "NotesTextSelection", default)]
    pub notes_text_selection: String,
    #[serde(rename = "CustomMetaData", default)]
    pub custom_metadata: Vec<CustomMetadataItem>,
}

/// Custom metadata items
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename = "MetaDataItem")]
pub struct CustomMetadataItem {
    #[serde(rename = "FieldID", default)]
    pub field_id: String,
    #[serde(rename = "Value", default)]
    pub value: String,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct Target {
    #[serde(rename = "@Type", default)]
    pub r#type: String,
    #[serde(rename = "@Notify", default)]
    pub notify: bool,
    #[serde(rename = "@ShowOverrun", default)]
    pub show_overrun: bool,
    #[serde(rename = "@ShowBuffer", default)]
    pub show_buffer: bool,
    #[serde(rename = "$value")]
    pub value: usize,
}

/// TextSettings
///
/// Contains current selection and any targets set
#[derive(Debug, Deserialize, PartialEq)]
pub struct TextSettings {
    #[serde(rename = "TextSelection", default)]
    pub text_selection: String,
    #[serde(rename = "Target", default)]
    pub target: Option<Target>,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct ItemKeywords {
    #[serde(rename = "KeywordId", default)]
    pub keyword_ids: Vec<usize>,
}

/// A binder item
///
/// Maybe folder, text or other content
#[derive(Debug, Deserialize, PartialEq)]
pub struct BinderItem {
    #[serde(rename = "@UUID", default)]
    pub uuid: Uuid,
    #[serde(rename = "@Type", default)]
    pub r#type: BinderItemType,
    #[serde(rename = "Title", default)]
    pub title: String,
    #[serde(rename = "MetaData", default)]
    pub metadata: Option<BinderItemMetadata>,
    #[serde(rename = "TextSettings", default)]
    pub text_settings: Option<TextSettings>,
    #[serde(rename = "Keywords", default)]
    pub keywords: Option<ItemKeywords>,
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

#[derive(Debug, Deserialize, PartialEq, Default)]
pub struct SearchSettings {
    #[serde(rename = "@Type")]
    pub r#type: String,
    #[serde(rename = "@Operator")]
    pub operator: String,
    #[serde(rename = "@Scope", default)]
    pub scope: String,
    #[serde(rename = "@CaseSensitive")]
    pub case_sensitive: bool,
    #[serde(rename = "@IgnoreDiacritics")]
    pub ignore_diacritics: bool,
}

#[derive(Debug, Deserialize, PartialEq, Default)]
pub struct Collection {
    #[serde(rename = "Title")]
    pub title: String,
    #[serde(rename = "@Type")]
    pub r#type: String,
    #[serde(rename = "@ID")]
    pub id: Uuid,
    #[serde(rename = "@Color")]
    pub color: String,
    #[serde(rename = "SearchSettings", default)]
    pub search_settings: SearchSettings,
}

/// Configured collections
#[derive(Debug, Deserialize, PartialEq, Default)]
pub struct Collections {
    #[serde(rename = "Collection")]
    pub collections: Vec<Collection>,
}

#[derive(Debug, Deserialize, PartialEq, Default)]
pub struct Keyword {
    #[serde(rename = "@ID")]
    pub id: usize,
    #[serde(rename = "@Color", default)]
    pub color: String,
    pub children: ChildKeywords,
}

#[derive(Debug, Deserialize, PartialEq, Default)]
pub struct ChildKeywords {
    pub keywords: Vec<Keyword>,
}

/// A reference to a binder item with
pub struct BinderItemLocation<'a> {
    pub item: &'a BinderItem,
    pub parents: Vec<&'a BinderItem>,
}

/// An iterator over binder items
pub struct BinderIterator<'a> {
    root_queue: VecDeque<&'a BinderItem>,
    stack: Vec<&'a BinderItem>,
}

impl<'a> BinderIterator<'a> {
    /// An iterator over the binder's items
    pub fn new(roots: Vec<&'a BinderItem>) -> BinderIterator<'a> {
        BinderIterator {
            root_queue: roots.into_iter().collect(),
            stack: vec![],
        }
    }

    pub fn new_from_root(root: &'a BinderItem) -> BinderIterator<'a> {
        BinderIterator {
            root_queue: VecDeque::from([root]),
            stack: vec![],
        }
    }
}

// impl<'a> Iterator for BinderIterator<'a> {
//     type Item = &'a BinderItem;

//     fn next(&mut self) -> Option<Self::Item> {
//         if let Some(item) = self.stack.pop() {
//             if !item.children.binder_items.is_empty() {
//                 self.stack.extend(item.children.binder_items.iter().rev());
//                 Some(item)
//             } else {
//                 Some(item)
//             }
//         } else {
//             None
//         }
//     }
// }

impl<'a> Iterator for BinderIterator<'a> {
    type Item = BinderItemLocation<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.stack.last().copied() {
            if !item.children.binder_items.is_empty() {
                self.stack.extend(item.children.binder_items.iter().rev());
                let parents = self.stack.clone();
                Some(BinderItemLocation { item, parents })
            } else {
                let parents = self.stack.clone();
                self.stack.pop();
                Some(BinderItemLocation { item, parents })
            }
        } else if let Some(item) = self.root_queue.pop_front() {
            self.stack.push(item);
            Some(BinderItemLocation {
                item,
                parents: vec![],
            })
        } else {
            None
        }
    }
}
