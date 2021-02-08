///! Scry mainline - parse and extract text snippets from Scrivener project
use std::fs::File;
use structopt::StructOpt;

pub mod annot;
pub mod bundle;
pub mod error;
pub mod extract;
pub mod options;
pub mod rtf;
pub mod scrivx;
pub mod tag;

use error::{Result, ScryError};

fn main() {
    let opts = options::Opt::from_args();
    if let Err(e) = try_main(&opts) {
        eprintln!("Error: {}", e);
    }
}

/// Run extraction capturing error for reporting
fn try_main(opts: &options::Opt) -> Result<()> {
    let extractor = create_extractor(opts)?;
    for text in extractor.iter() {
        println!("{}", text);
    }
    Ok(())
}

/// Create an extractor based on command line options
fn create_extractor(opts: &options::Opt) -> Result<extract::Extractor> {
    // Find the project file
    let scrivx = File::open(opts.project())?;
    let directory = opts
        .project()
        .parent()
        .ok_or(ScryError::CannotLocateBundle)?;

    // Parse project
    let project = scrivx::ScrivenerProject::parse(scrivx)?;

    // Prepare bundle
    let bundle = bundle::Bundle::new(directory);

    Ok(extract::Extractor::new(
        project,
        bundle,
        opts.folder_specs(),
        opts.content_specs(),
    ))
}
