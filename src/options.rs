//! Scry command line options
use crate::extract::{ContentSpec, FolderSpec};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "scry", about = "Extract content from scrivener project")]
pub struct Opt {
    /// Include the draft folder
    #[structopt(short, long)]
    draft: bool,

    /// Include the research folder
    #[structopt(short, long)]
    research: bool,

    /// Include the trash folder
    #[structopt(long)]
    trash: bool,

    /// Include top-level folder
    #[structopt(short = "F", long = "folder")]
    folders: Vec<String>,

    /// Include paragraphs from item content
    #[structopt(short, long)]
    content: bool,

    /// Include item titles
    #[structopt(short, long)]
    titles: bool,

    /// Include paragraphs from item notes
    #[structopt(short, long)]
    notes: bool,

    /// Include inline comments from item content
    #[structopt(short, long)]
    inlines: bool,

    /// Include out-of-line item comments
    #[structopt(short = "m", long)]
    comments: bool,

    /// Include synopses
    #[structopt(short = "s", long)]
    synopses: bool,

    /// Project, either a .scrivx file or a project bundle folder
    /// (containing a .scrivx file)
    #[structopt(name = "PROJECT")]
    project: PathBuf,
}

impl Opt {
    pub fn project(&self) -> &Path {
        &self.project
    }

    /// Return the folders to include in the output
    pub fn folder_specs(&self) -> HashSet<FolderSpec> {
        let mut folder_specs = HashSet::new();
        if self.draft {
            folder_specs.insert(FolderSpec::DraftFolder);
        }
        if self.research {
            folder_specs.insert(FolderSpec::ResearchFolder);
        }
        if self.trash {
            folder_specs.insert(FolderSpec::TrashFolder);
        }
        for s in &self.folders {
            folder_specs.insert(FolderSpec::NamedFolder(s.to_string()));
        }
        if folder_specs.is_empty() {
            folder_specs.insert(FolderSpec::DraftFolder);
        }
        folder_specs
    }

    /// The types of content to extract for each item
    pub fn content_specs(&self) -> HashSet<ContentSpec> {
        let mut content_specs = HashSet::new();
        if self.titles {
            content_specs.insert(ContentSpec::Title);
        }
        if self.synopses {
            content_specs.insert(ContentSpec::Synopsis);
        }
        if self.content {
            content_specs.insert(ContentSpec::Content);
        }
        if self.notes {
            content_specs.insert(ContentSpec::Notes);
        }
        if self.inlines {
            content_specs.insert(ContentSpec::Inlines);
        }
        if self.comments {
            content_specs.insert(ContentSpec::Comments);
        }
        if content_specs.is_empty() {
            content_specs.insert(ContentSpec::Content);
        }
        content_specs
    }

    /// Find a .scrivx file in specified folder
    fn find_scrivx_child(path: &Path) -> Option<PathBuf> {
        let dir = path.read_dir().ok()?;

        for entry in dir {
            let f = entry.ok()?;
            if f.path().extension() == Some(OsStr::new("scrivx")) {
                return Some(f.path());
            }
        }

        None
    }

    /// Identify the project file implied by the project argument
    pub fn project_file(&self) -> Option<PathBuf> {
        if self.project.is_file() {
            Some(self.project.clone())
        } else if self.project.is_dir() {
            Self::find_scrivx_child(&self.project)
        } else {
            None
        }
    }
}
