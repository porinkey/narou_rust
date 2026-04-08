use crate::model::{NovelRecord, Section, Subtitle, Toc};
use anyhow::{Context, Result};
use regex::Regex;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const DATABASE_DIR: &str = ".narou";
const DATABASE_FILE: &str = "database.yaml";
const ARCHIVE_ROOT: &str = "小説データ";
const SECTION_DIR: &str = "本文";
const RAW_DIR: &str = "raw";

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join(DATABASE_DIR))?;
        fs::create_dir_all(root.join(ARCHIVE_ROOT))?;
        Ok(Self { root })
    }

    pub fn load_database(&self) -> Result<BTreeMap<u64, NovelRecord>> {
        self.load_yaml(self.database_path())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn save_database(&self, database: &BTreeMap<u64, NovelRecord>) -> Result<()> {
        self.save_yaml(self.database_path(), database)
    }

    pub fn find_record<'a>(
        &self,
        database: &'a BTreeMap<u64, NovelRecord>,
        target: &crate::model::DownloadTarget,
    ) -> Option<&'a NovelRecord> {
        match target {
            crate::model::DownloadTarget::Id(id) => database.get(id),
            crate::model::DownloadTarget::Ncode(ncode) => {
                database.values().find(|record| record.toc_url.ends_with(&format!("{ncode}/")))
            }
            crate::model::DownloadTarget::Url(url) => database.values().find(|record| record.toc_url == *url),
        }
    }

    pub fn novel_dir(&self, sitename: &str, file_title: &str) -> PathBuf {
        self.root.join(ARCHIVE_ROOT).join(sitename).join(file_title)
    }

    pub fn load_toc(&self, sitename: &str, file_title: &str) -> Result<Toc> {
        self.load_yaml(self.novel_dir(sitename, file_title).join("toc.yaml"))
    }

    pub fn save_toc(&self, sitename: &str, file_title: &str, toc: &Toc) -> Result<()> {
        self.save_yaml(self.novel_dir(sitename, file_title).join("toc.yaml"), toc)
    }

    pub fn save_section(
        &self,
        sitename: &str,
        file_title: &str,
        subtitle: &Subtitle,
        section: &Section,
    ) -> Result<()> {
        let path = self
            .novel_dir(sitename, file_title)
            .join(SECTION_DIR)
            .join(format!("{} {}.yaml", subtitle.index, subtitle.file_subtitle));
        self.save_yaml(path, section)
    }

    pub fn save_raw_html(&self, sitename: &str, file_title: &str, subtitle: &Subtitle, html: &str) -> Result<()> {
        let path = self
            .novel_dir(sitename, file_title)
            .join(RAW_DIR)
            .join(format!("{} {}.html", subtitle.index, subtitle.file_subtitle));
        self.write_text(path, html)
    }

    pub fn load_section(&self, sitename: &str, file_title: &str, subtitle: &Subtitle) -> Result<Section> {
        self.load_yaml(
            self.novel_dir(sitename, file_title)
                .join(SECTION_DIR)
                .join(format!("{} {}.yaml", subtitle.index, subtitle.file_subtitle)),
        )
    }

    pub fn section_exists(&self, sitename: &str, file_title: &str, subtitle: &Subtitle) -> bool {
        self.novel_dir(sitename, file_title)
            .join(SECTION_DIR)
            .join(format!("{} {}.yaml", subtitle.index, subtitle.file_subtitle))
            .exists()
    }

    pub fn workspace_path(&self, relative_name: &str) -> PathBuf {
        self.root.join(relative_name)
    }

    pub fn write_workspace_text(&self, relative_name: &str, value: &str) -> Result<()> {
        self.write_text(self.workspace_path(relative_name), value)
    }

    pub fn write_text_at(&self, path: &Path, value: &str) -> Result<()> {
        self.write_text(path, value)
    }

    pub fn remove_novel_dir(&self, sitename: &str, file_title: &str) -> Result<()> {
        let path = self.novel_dir(sitename, file_title);
        if path.exists() {
            fs::remove_dir_all(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
        Ok(())
    }

    pub fn calculate_novel_length(
        &self,
        sitename: &str,
        file_title: &str,
        subtitles: &[Subtitle],
    ) -> Result<usize> {
        let mut total = 0usize;
        for subtitle in subtitles {
            let section = self.load_section(sitename, file_title, subtitle)?;
            total += html_text_length(&section.element.introduction);
            total += html_text_length(&section.element.body);
            total += html_text_length(&section.element.postscript);
        }
        Ok(total)
    }

    fn database_path(&self) -> PathBuf {
        self.root.join(DATABASE_DIR).join(DATABASE_FILE)
    }

    fn load_yaml<T>(&self, path: impl AsRef<Path>) -> Result<T>
    where
        T: DeserializeOwned + Default,
    {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(T::default());
        }
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let value = serde_yaml::from_str::<T>(&text)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(value)
    }

    fn save_yaml<T>(&self, path: impl AsRef<Path>, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_yaml::to_string(value)?;
        fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    fn write_text(&self, path: impl AsRef<Path>, value: &str) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, value).with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }
}

fn html_text_length(input: &str) -> usize {
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("valid regex");
    tag_re
        .replace_all(input, "")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .chars()
        .count()
}
