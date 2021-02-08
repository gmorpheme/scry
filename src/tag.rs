///! Adaptation for stripping style tags form Scrivener content
///!
///! Tags are the style related elements like the following that
///! Scrivener inserts into the RTF:
///!
///! '<$ScrKeepWithNext><$Scr_H::1><$Scr_Ps::0>blah<!$Scr_H::1><!$Scr_Ps::0>'
use regex::Regex;

const SCRIVENER_TAG: &str = r#"<!?\$Scr.*?>"#;

pub fn strip_tags(line: String) -> String {
    Regex::new(SCRIVENER_TAG)
        .unwrap()
        .replace_all(&line, "")
        .into_owned()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    pub fn test() {
        let s =
            "<$ScrKeepWithNext><$Scr_H::1><$Scr_Ps::0>blah<!$Scr_H::1><!$Scr_Ps::0>".to_string();
        assert_eq!(strip_tags(s), "blah");
    }

    #[test]
    pub fn test_2() {
        let s = "<$Scr_Ps::0>25th April 1955".to_string();
        assert_eq!(strip_tags(s), "25th April 1955");
    }
}
