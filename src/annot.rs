//! Iterator adaptor to handle Scrivener's ugly annotation tags
//!
//! They're kind of another layer of RTF-like syntax embedded in the
//! text that the RTF specifies (so escaped in the eventual RTF).
//! However, the control words are invalid (containing underscores
//! and equals) so they can't be tokenised by a standard RTF
//! tokeniser.
//!
//! Furthermore we may have split the "group" across several lines
//! during our RTF snipperation. So we have to be quite hacky here.

/// Adapted iterator that strips annotations from the underlying iterator
pub fn skip_annotations<T>(source: T) -> AnnotationAdapter<T>
where
    T: Iterator<Item = String>,
{
    AnnotationAdapter::new(source, true, false)
}

/// Adapted iterator that selects only annotations from the underlying iterator
pub fn only_annotations<T>(source: T) -> AnnotationAdapter<T>
where
    T: Iterator<Item = String>,
{
    AnnotationAdapter::new(source, false, true)
}

/// Adapts an rtf::ParagraphIterator to remove or retain annotations.
pub struct AnnotationAdapter<T>
where
    T: Iterator<Item = String>,
{
    /// Line source
    source: itertools::PutBack<T>,
    /// Whether we are in an annotation at the start of the next line
    in_annotation: bool,
    /// Whether to forward on normal content
    output_content: bool,
    /// Whether to forward on annotation content
    output_annot: bool,
}

const OPEN: &str = r#"{\Scrv_annot"#;
const OPEN_END: &str = r#"\text="#;
const CLOSE: &str = r#"\end_Scrv_annot}"#;

impl<T: Iterator<Item = String>> AnnotationAdapter<T> {
    /// Construct an annotation-sensitive iterator that outputs
    /// content and annotations as specified
    pub fn new(source: T, output_content: bool, output_annot: bool) -> Self {
        AnnotationAdapter {
            source: itertools::put_back(source),
            in_annotation: false,
            output_content,
            output_annot,
        }
    }

    /// Return the portion of line prior to whatever terminates the
    /// current mode, potentially putting back the rest.
    ///
    /// None does not indicate an exhausted iterator but a chunk
    /// incompatible with output settings
    fn take_chunk<'a>(&mut self, line: &'a str) -> Option<&'a str> {
        if self.in_annotation {
            let annot = match line.find(CLOSE) {
                Some(idx) => {
                    self.in_annotation = false;
                    self.source
                        .put_back((&line[(idx + CLOSE.len())..]).to_string());
                    &line[..idx]
                }
                None => line,
            };

            if self.output_annot && !annot.is_empty() {
                Some(annot)
            } else {
                None
            }
        } else {
            let content = match line.find(OPEN) {
                Some(start) => {
                    let end = line
                        .find(OPEN_END)
                        .expect("Unsupported: annotation split open across lines");
                    self.in_annotation = true;
                    self.source
                        .put_back((&line[(end + OPEN_END.len())..]).to_string());
                    &line[..start]
                }
                None => line,
            };

            if self.output_content && !content.is_empty() {
                Some(content)
            } else {
                None
            }
        }
    }
}

impl<T> Iterator for AnnotationAdapter<T>
where
    T: Iterator<Item = String>,
{
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(line) = self.source.next() {
            if let Some(chunk) = self.take_chunk(&line) {
                return Some(chunk.to_string());
            }
        }

        None
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    pub fn test_simple() {
        let source = vec!["one".to_string(), "two".to_string()];
        let lines: Vec<_> = AnnotationAdapter::new(source.into_iter(), true, false).collect();
        assert_eq!(lines, &["one", "two"]);
    }

    #[test]
    pub fn test_empty() {
        let source = vec!["one".to_string(), "two".to_string()];
        let lines: Vec<_> = AnnotationAdapter::new(source.into_iter(), false, false).collect();
        assert!(lines.is_empty());
    }

    #[test]
    pub fn test_strips_annot() {
        let source = vec![r#"{\Scrv_annot \color={\R=0.148574\G=0.477381\B=0.267573} \text=this is an annotation\end_Scrv_annot}This is normal content."#.to_string()];
        let lines: Vec<_> = AnnotationAdapter::new(source.into_iter(), true, false).collect();
        assert_eq!(lines, &["This is normal content."]);
    }

    #[test]
    pub fn test_strips_content() {
        let source = vec![r#"{\Scrv_annot \color={\R=0.148574\G=0.477381\B=0.267573} \text=this is an annotation\end_Scrv_annot}This is normal content."#.to_string()];
        let lines: Vec<_> = AnnotationAdapter::new(source.into_iter(), false, true).collect();
        assert_eq!(lines, &["this is an annotation"]);
    }

    #[test]
    pub fn test_splits_annotation_and_content() {
        let source = vec![r#"{\Scrv_annot \color={\R=0.148574\G=0.477381\B=0.267573} \text=this is an annotation\end_Scrv_annot}This is normal content."#.to_string()];
        let lines: Vec<_> = AnnotationAdapter::new(source.into_iter(), true, true).collect();
        assert_eq!(lines, &["this is an annotation", "This is normal content."]);
    }
}
