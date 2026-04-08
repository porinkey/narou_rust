use anyhow::{anyhow, bail, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadTarget {
    Ncode(String),
    Url(String),
    Id(u64),
}

impl DownloadTarget {
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            bail!("target is empty");
        }
        let ncode_re = Regex::new(r"(?i)^n\d+[a-z]+$").expect("valid regex");
        if ncode_re.is_match(trimmed) {
            return Ok(Self::Ncode(trimmed.to_ascii_lowercase()));
        }
        if let Ok(id) = trimmed.parse::<u64>() {
            return Ok(Self::Id(id));
        }
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return Ok(Self::Url(trimmed.to_string()));
        }
        Err(anyhow!("unsupported target: {trimmed}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NovelRecord {
    pub id: u64,
    pub author: String,
    pub title: String,
    pub file_title: String,
    pub toc_url: String,
    pub sitename: String,
    pub novel_type: u8,
    pub end: bool,
    pub all_episodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_arrivals_date: Option<String>,
    #[serde(default)]
    pub use_subdirectory: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub general_firstup: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub novelupdated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub general_lastup: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<usize>,
    #[serde(default)]
    pub suspend: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub general_all_no: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Toc {
    pub title: String,
    pub author: String,
    pub toc_url: String,
    pub story: String,
    pub subtitles: Vec<Subtitle>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Subtitle {
    pub index: String,
    pub href: String,
    pub chapter: String,
    pub subchapter: String,
    pub subtitle: String,
    pub file_subtitle: String,
    pub subdate: String,
    pub subupdate: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Section {
    pub chapter: String,
    pub subchapter: String,
    pub subtitle: String,
    pub element: SectionElement,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SectionElement {
    pub data_type: String,
    pub introduction: String,
    pub postscript: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedNovel {
    pub toc_url: String,
    pub info_url: String,
    pub base_url: String,
}

#[derive(Debug, Clone)]
pub struct ParsedNovel {
    pub title: String,
    pub author: String,
    pub sitename: String,
    pub story: String,
    pub toc_url: String,
    pub episodes: Vec<Subtitle>,
    pub novel_type: u8,
    pub end: bool,
    pub general_firstup: Option<String>,
    pub novelupdated_at: Option<String>,
    pub general_lastup: Option<String>,
    pub length: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::DownloadTarget;

    #[test]
    fn parses_ncode() {
        let target = DownloadTarget::parse("N9669BK").unwrap();
        match target {
            DownloadTarget::Ncode(value) => assert_eq!(value, "n9669bk"),
            _ => panic!("unexpected target"),
        }
    }
}
