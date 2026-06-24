//! Scry mainline - parse and extract text snippets from Scrivener project

pub mod annot;
pub mod bundle;
pub mod error;
pub mod extract;
pub mod markdown;
pub mod options;
pub mod rtf;
pub mod scrivx;
pub mod tag;

use std::fs::File;

use error::{Result, ScryError};
use extract::binder_iterator;
use extract::JsonItemiser;
use structopt::StructOpt;

fn main() {
    let opts = options::Opt::from_args();
    if let Err(e) = try_main(&opts) {
        eprintln!("Error: {}", e);
    }
}

/// Infer project name from the scrivx file path
fn project_name(project_file: &std::path::Path) -> String {
    // Walk up to find the .scriv bundle directory
    if let Some(parent) = project_file.parent() {
        if let Some(name) = parent.file_stem() {
            return name.to_string_lossy().to_string();
        }
    }
    "project".to_string()
}

/// Run extraction capturing error for reporting
fn try_main(opts: &options::Opt) -> Result<()> {
    let project_file = opts.project_file().ok_or(ScryError::CannotLocateScrivx)?;
    let scrivx = File::open(&project_file)?;
    let directory = project_file.parent().ok_or(ScryError::CannotLocateBundle)?;
    let project = scrivx::ScrivenerProject::parse(scrivx)?;
    let bundle = bundle::Bundle::new(directory);

    if opts.markdown() {
        let output_dir = opts.output_dir().ok_or_else(|| {
            ScryError::CannotLocateBundle // reuse error; --output-dir is required by structopt
        })?;
        let name = project_name(&project_file);
        let folder_specs = opts.folder_specs();
        let content_specs = opts.content_specs();

        if opts.split() {
            markdown::emit_split(
                &project, &bundle, output_dir, &name,
                &folder_specs, &content_specs, opts.asset_path(),
            )?;
        } else {
            markdown::emit_single_file(
                &project, &bundle, output_dir, &name,
                &folder_specs, &content_specs, opts.asset_path(),
            )?;
        }
    } else if opts.itemise() {
        let items = binder_iterator(&project, opts.folder_specs());
        let mut itemiser = JsonItemiser::new(opts.content_specs());
        for item_location in items {
            let folder = bundle.binder_item_content(&item_location.item.uuid);
            itemiser.consume_item(&item_location, &folder)?;
        }
        itemiser.write_to_stdout()?;
    } else {
        let extractor =
            extract::Extractor::new(project, bundle, opts.folder_specs(), opts.content_specs());
        for text in extractor.iter() {
            println!("{}", text);
        }
    }

    Ok(())
}
