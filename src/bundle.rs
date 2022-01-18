//! The file system directory structure for a scrivener project
use std::convert::AsRef;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The file system directory structure for a scrivener project
///
/// This includes all RTF and image content.
pub struct Bundle {
    /// The root directory (containing .scrivx file)
    root: PathBuf,
}

impl Bundle {
    /// Construct a bundle from base directory
    pub fn new<T>(root: T) -> Self
    where
        T: AsRef<Path>,
    {
        Bundle {
            root: root.as_ref().to_owned(),
        }
    }

    /// The root directory of the bundle
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Content folder for a binder item
    pub fn binder_item_folder(&self, binder_item: &Uuid) -> PathBuf {
        let mut path = self.root.clone();
        path.push("Files");
        path.push("Data");
        path.push(
            binder_item
                .to_hyphenated_ref()
                .encode_upper(&mut Uuid::encode_buffer()),
        );
        path
    }

    /// Retrieve BinderItemFolder for specified content item
    pub fn binder_item_content(&self, binder_item: &Uuid) -> BinderItemFolder {
        let folder = self.binder_item_folder(binder_item);
        BinderItemFolder::new(folder)
    }
}

/// Find the content file under `folder` if it exists
fn find_content(folder: &Path) -> Option<PathBuf> {
    for entry in folder.read_dir().ok()?.flatten() {
        let path = entry.path();
        let stem = path.file_stem();
        let ext = path.extension();
        if stem == Some(OsStr::new("content")) && ext != Some(OsStr::new("comments")) {
            return Some(entry.path());
        }
    }
    None
}

/// Return the specified file under the folder if it exists
fn existing_child(folder: &Path, name: &str) -> Option<PathBuf> {
    let mut path = folder.to_path_buf();
    path.push(name);
    if path.is_file() {
        Some(path)
    } else {
        None
    }
}

/// Access to key paths for a binder item
pub struct BinderItemFolder {
    /// The item's folder
    folder: PathBuf,
    /// The item's content file
    content: Option<PathBuf>,
    /// Path of item notes (RTF)
    notes: Option<PathBuf>,
    /// Path of synopsis (text)
    synopsis: Option<PathBuf>,
    /// Path of comments file
    comments: Option<PathBuf>,
}

impl BinderItemFolder {
    pub fn new(folder: PathBuf) -> Self {
        let content = find_content(&folder);
        let notes = existing_child(&folder, "notes.rtf");
        let synopsis = existing_child(&folder, "synopsis.txt");
        let comments = existing_child(&folder, "content.comments");

        BinderItemFolder {
            folder,
            content,
            notes,
            synopsis,
            comments,
        }
    }

    /// Path to item folder
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Path to item content
    pub fn content(&self) -> Option<&Path> {
        self.content.as_deref()
    }

    /// Path to item notes
    pub fn notes(&self) -> Option<&Path> {
        self.notes.as_deref()
    }

    /// Path to item synopsis
    pub fn synopsis(&self) -> Option<&Path> {
        self.synopsis.as_deref()
    }

    /// Path to item comments
    pub fn comments(&self) -> Option<&Path> {
        self.comments.as_deref()
    }
}
