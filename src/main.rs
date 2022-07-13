//! Scry mainline - parse and extract text snippets from Scrivener project

pub mod annot;
pub mod bundle;
pub mod error;
pub mod extract;
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

/// Run extraction capturing error for reporting
fn try_main(opts: &options::Opt) -> Result<()> {
    let project_file = opts.project_file().ok_or(ScryError::CannotLocateScrivx)?;
    let scrivx = File::open(&project_file)?;
    let directory = project_file.parent().ok_or(ScryError::CannotLocateBundle)?;
    let project = scrivx::ScrivenerProject::parse(scrivx)?;
    let bundle = bundle::Bundle::new(directory);

    if opts.itemise() {
        let items = binder_iterator(&project, opts.folder_specs());
        let mut itemiser = JsonItemiser::new(opts.content_specs());
        for item in items {
            let folder = bundle.binder_item_content(&item.uuid);
            itemiser.consume_item(item, &folder)?;
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
