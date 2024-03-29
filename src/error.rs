use json;
use quick_xml::DeError;
use rtf_grimoire::tokenizer::ParseError;
use std::io;
use thiserror::Error;

/// Error while processing scrivener project
#[derive(Error, Debug)]
pub enum ScryError {
    #[error(transparent)]
    IOError(#[from] io::Error),
    #[error("failed to parse RTF: {0}")]
    RtfParse(ParseError),
    #[error("failed to parse XML: {0}")]
    XmlParse(#[from] DeError),
    #[error("failed to format JSON: {0}")]
    JsonError(#[from] json::Error),
    #[error("unable to locate bundle containing project")]
    CannotLocateBundle,
    #[error("unable to locate .scrivx project file")]
    CannotLocateScrivx,
}

/// Scry result
pub type Result<T> = std::result::Result<T, ScryError>;

impl From<ParseError> for ScryError {
    fn from(e: ParseError) -> Self {
        ScryError::RtfParse(e)
    }
}
