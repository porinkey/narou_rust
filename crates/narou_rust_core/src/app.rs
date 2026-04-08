use crate::config::AppConfig;
use crate::convert::{create_epub, EpubSummary};
use crate::diagnostic::{format_error_report, ErrorContext};
use crate::kakuyomu::KakuyomuClient;
use crate::model::{DownloadTarget, NovelRecord, ParsedNovel, Toc};
use crate::storage::Workspace;
use crate::syosetu::SyosetuClient;
use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Local, NaiveDateTime, TimeZone};
use std::collections::BTreeMap;
use std::path::Path;
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone)]
pub struct DownloadSummary {
    pub record: NovelRecord,
    pub episodes_downloaded: usize,
    pub epub: Option<EpubSummary>,
}

#[derive(Debug, Clone)]
pub struct BatchDownloadItem {
    pub target: String,
    pub result: Result<DownloadSummary, String>,
}

#[derive(Debug, Clone)]
pub struct BatchDownloadSummary {
    pub input_file: String,
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub success_file: String,
    pub failed_file: String,
    pub summary_file: String,
    pub items: Vec<BatchDownloadItem>,
}

#[derive(Debug, Clone)]
pub struct InspectSummary {
    pub record: NovelRecord,
    pub workspace: String,
    pub novel_dir: String,
    pub toc_path: String,
    pub txt_path: String,
    pub raw_dir: String,
    pub section_dir: String,
    pub epub_files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RemoveSummary {
    pub record: NovelRecord,
    pub removed_files: bool,
}

pub struct App {
    workspace: Workspace,
    config: AppConfig,
    syosetu: SyosetuClient,
    kakuyomu: KakuyomuClient,
}

impl App {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Result<Self> {
        let workspace = Workspace::new(root)?;
        let config = AppConfig::load(workspace.root())?;
        Ok(Self {
            syosetu: SyosetuClient::new(config.clone())?,
            kakuyomu: KakuyomuClient::new(config.clone())?,
            workspace,
            config,
        })
    }

    pub fn list_records(&self) -> Result<Vec<NovelRecord>> {
        let database = self.workspace.load_database()?;
        Ok(database.into_values().collect())
    }

    pub fn inspect(&self, target: &DownloadTarget) -> Result<InspectSummary> {
        let database = self.workspace.load_database()?;
        let record = self
            .workspace
            .find_record(&database, target)
            .cloned()
            .context("record not found")?;
        let novel_dir = self.workspace.novel_dir(&record.sitename, &record.file_title);
        let toc_path = novel_dir.join("toc.yaml");
        let txt_path = novel_dir.join(format!("{}.txt", record.file_title));
        let raw_dir = novel_dir.join("raw");
        let section_dir = novel_dir.join("本文");
        let epub_files = if novel_dir.exists() {
            std::fs::read_dir(&novel_dir)?
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| {
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext.eq_ignore_ascii_case("epub"))
                        == Some(true)
                })
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        Ok(InspectSummary {
            record,
            workspace: self.workspace.root().display().to_string(),
            novel_dir: novel_dir.display().to_string(),
            toc_path: toc_path.display().to_string(),
            txt_path: txt_path.display().to_string(),
            raw_dir: raw_dir.display().to_string(),
            section_dir: section_dir.display().to_string(),
            epub_files,
        })
    }

    pub fn convert_saved(
        &self,
        target: &DownloadTarget,
        aozora_dir: Option<&Path>,
    ) -> Result<DownloadSummary> {
        let database = self.workspace.load_database()?;
        let record = self
            .workspace
            .find_record(&database, target)
            .cloned()
            .context("record not found")?;
        let epub = Some(create_epub(
            &self.workspace,
            &record.sitename,
            &record.file_title,
            aozora_dir,
        )?);
        Ok(DownloadSummary {
            record,
            episodes_downloaded: 0,
            epub,
        })
    }

    pub fn remove(&self, target: &DownloadTarget, remove_files: bool) -> Result<RemoveSummary> {
        let mut database = self.workspace.load_database()?;
        let record = self
            .workspace
            .find_record(&database, target)
            .cloned()
            .context("record not found")?;
        database.remove(&record.id);
        self.workspace.save_database(&database)?;
        if remove_files {
            self.workspace
                .remove_novel_dir(&record.sitename, &record.file_title)?;
        }
        Ok(RemoveSummary {
            record,
            removed_files: remove_files,
        })
    }

    pub async fn download(&self, target: &DownloadTarget, save_raw: bool) -> Result<DownloadSummary> {
        let mut database = self.workspace.load_database()?;
        let summary = self
            .download_into_database(&mut database, target, save_raw, true, None)
            .await?;
        self.workspace.save_database(&database)?;
        Ok(summary)
    }

    pub async fn download_with_epub(
        &self,
        target: &DownloadTarget,
        save_raw: bool,
        aozora_dir: Option<&Path>,
    ) -> Result<DownloadSummary> {
        let mut database = self.workspace.load_database()?;
        let summary = self
            .download_into_database(&mut database, target, save_raw, true, aozora_dir)
            .await?;
        self.workspace.save_database(&database)?;
        Ok(summary)
    }

    pub async fn update(
        &self,
        targets: Vec<DownloadTarget>,
        save_raw: bool,
        aozora_dir: Option<&Path>,
    ) -> Result<Vec<DownloadSummary>> {
        let mut database = self.workspace.load_database()?;
        let resolved_targets = if targets.is_empty() {
            database
                .values()
                .map(|record| DownloadTarget::Url(record.toc_url.clone()))
                .collect::<Vec<_>>()
        } else {
            targets
        };

        let mut summaries = Vec::new();
        for (index, target) in resolved_targets.iter().enumerate() {
            let summary = self
                .download_into_database(&mut database, &target, save_raw, false, aozora_dir)
                .await?;
            summaries.push(summary);
            if index + 1 < resolved_targets.len() && self.config.update_interval_secs > 0.0 {
                sleep(Duration::from_secs_f64(self.config.update_interval_secs)).await;
            }
        }

        self.workspace.save_database(&database)?;
        Ok(summaries)
    }

    pub async fn batch_download(
        &self,
        input: &Path,
        save_raw: bool,
        aozora_dir: Option<&Path>,
    ) -> Result<BatchDownloadSummary> {
        let text = std::fs::read_to_string(input)
            .with_context(|| format!("failed to read {}", input.display()))?;
        let mut database = self.workspace.load_database()?;
        let mut items = Vec::new();
        let mut success_targets = Vec::new();
        let mut failed_targets = Vec::new();

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let parsed_target = DownloadTarget::parse(trimmed);
            match parsed_target {
                Ok(target) => match self
                    .download_into_database(&mut database, &target, save_raw, true, aozora_dir)
                    .await
                {
                    Ok(summary) => {
                        success_targets.push(trimmed.to_string());
                        items.push(BatchDownloadItem {
                            target: trimmed.to_string(),
                            result: Ok(summary),
                        });
                    }
                    Err(error) => {
                        failed_targets.push(trimmed.to_string());
                        items.push(BatchDownloadItem {
                            target: trimmed.to_string(),
                            result: Err(format_error_report(
                                &error,
                                &ErrorContext::new()
                                    .command("batch-download")
                                    .target(trimmed)
                                    .workspace(self.workspace.root().display().to_string()),
                            )),
                        });
                    }
                },
                Err(error) => {
                    failed_targets.push(trimmed.to_string());
                    items.push(BatchDownloadItem {
                        target: trimmed.to_string(),
                        result: Err(format_error_report(
                            &error,
                            &ErrorContext::new()
                                .command("batch-download")
                                .target(trimmed)
                                .workspace(self.workspace.root().display().to_string()),
                        )),
                    });
                }
            }
        }

        self.workspace.save_database(&database)?;

        let success_file = "batch_download_success.txt";
        let failed_file = "batch_download_failed.txt";
        let summary_file = "batch_download_summary.txt";

        self.workspace
            .write_workspace_text(success_file, &join_lines(&success_targets))?;
        self.workspace
            .write_workspace_text(failed_file, &join_lines(&failed_targets))?;

        let summary = BatchDownloadSummary {
            input_file: input.display().to_string(),
            total: items.len(),
            success: success_targets.len(),
            failed: failed_targets.len(),
            success_file: self.workspace.workspace_path(success_file).display().to_string(),
            failed_file: self.workspace.workspace_path(failed_file).display().to_string(),
            summary_file: self.workspace.workspace_path(summary_file).display().to_string(),
            items,
        };

        self.workspace
            .write_workspace_text(summary_file, &summary.to_text())?;

        Ok(summary)
    }

    async fn download_into_database(
        &self,
        database: &mut BTreeMap<u64, NovelRecord>,
        target: &DownloadTarget,
        save_raw: bool,
        _fail_if_already_exists: bool,
        aozora_dir: Option<&Path>,
    ) -> Result<DownloadSummary> {
        let normalized_target = match target {
            DownloadTarget::Id(_) => {
                let record = self
                    .workspace
                    .find_record(database, target)
                    .context("record not found for id target")?;
                DownloadTarget::Url(record.toc_url.clone())
            }
            _ => target.clone(),
        };

        let existing = self.workspace.find_record(database, &normalized_target).cloned();
        let site_kind = detect_site_kind(&normalized_target, existing.as_ref());
        let (parsed, resolved) = match site_kind {
            SiteKind::Kakuyomu => {
                let resolved = self.kakuyomu.resolve_target(&normalized_target)?;
                let parsed = self.kakuyomu.fetch_novel(&resolved).await?;
                (parsed, resolved)
            }
            SiteKind::Syosetu => {
                let resolved = self.syosetu.resolve_target(&normalized_target)?;
                let parsed = self.syosetu.fetch_novel(&resolved).await?;
                (parsed, resolved)
            }
        };
        let file_title = build_file_title(existing.as_ref(), &parsed, &resolved.toc_url);

        let sitename = parsed.sitename.clone();
        let old_toc = existing
            .as_ref()
            .and_then(|record| self.workspace.load_toc(&record.sitename, &record.file_title).ok());

        let changed = changed_episodes(old_toc.as_ref(), &parsed, &sitename, &file_title, &self.workspace);
        for subtitle in &changed {
            let (section, raw_html) = match site_kind {
                SiteKind::Kakuyomu => self.kakuyomu.fetch_section(&parsed.toc_url, subtitle).await?,
                SiteKind::Syosetu => self.syosetu.fetch_section(&parsed.toc_url, subtitle).await?,
            };
            self.workspace.save_section(&sitename, &file_title, subtitle, &section)?;
            if save_raw {
                self.workspace.save_raw_html(&sitename, &file_title, subtitle, &raw_html)?;
            }
        }

        let toc = Toc {
            title: parsed.title.clone(),
            author: parsed.author.clone(),
            toc_url: parsed.toc_url.clone(),
            story: parsed.story.clone(),
            subtitles: parsed.episodes.clone(),
        };
        self.workspace.save_toc(&sitename, &file_title, &toc)?;
        let record = build_record(
            database,
            existing.as_ref(),
            &parsed,
            &file_title,
            resolved.toc_url.clone(),
            self.workspace
                .calculate_novel_length(&sitename, &file_title, &parsed.episodes)
                .ok(),
        );
        database.insert(record.id, record.clone());
        let epub = if let Some(aozora_dir) = aozora_dir {
            Some(create_epub(&self.workspace, &sitename, &file_title, Some(aozora_dir))?)
        } else {
            None
        };

        Ok(DownloadSummary {
            record,
            episodes_downloaded: changed.len(),
            epub,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum SiteKind {
    Syosetu,
    Kakuyomu,
}

fn detect_site_kind(target: &DownloadTarget, existing: Option<&NovelRecord>) -> SiteKind {
    if KakuyomuClient::supports(target) {
        return SiteKind::Kakuyomu;
    }
    if let Some(record) = existing {
        if record.toc_url.contains("kakuyomu.jp/works/") {
            return SiteKind::Kakuyomu;
        }
    }
    SiteKind::Syosetu
}

impl BatchDownloadSummary {
    pub fn to_text(&self) -> String {
        [
            format!("input_file: {}", self.input_file),
            format!("total: {}", self.total),
            format!("success: {}", self.success),
            format!("failed: {}", self.failed),
            format!("success_file: {}", self.success_file),
            format!("failed_file: {}", self.failed_file),
        ]
        .join("\n")
    }
}

impl InspectSummary {
    pub fn to_text(&self) -> String {
        let mut lines = vec![
            format!("id: {}", self.record.id),
            format!("site: {}", self.record.sitename),
            format!("title: {}", self.record.title),
            format!("author: {}", self.record.author),
            format!("toc_url: {}", self.record.toc_url),
            format!("episodes: {}", self.record.all_episodes),
            format!("workspace: {}", self.workspace),
            format!("novel_dir: {}", self.novel_dir),
            format!("toc_path: {}", self.toc_path),
            format!("txt_path: {}", self.txt_path),
            format!("raw_dir: {}", self.raw_dir),
            format!("section_dir: {}", self.section_dir),
        ];
        if self.epub_files.is_empty() {
            lines.push(String::from("epub_files: none"));
        } else {
            lines.push(String::from("epub_files:"));
            lines.extend(self.epub_files.iter().map(|path| format!("- {}", path)));
        }
        lines.join("\n")
    }
}

fn join_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn build_record(
    database: &BTreeMap<u64, NovelRecord>,
    existing: Option<&NovelRecord>,
    parsed: &ParsedNovel,
    file_title: &str,
    toc_url: String,
    calculated_length: Option<usize>,
) -> NovelRecord {
    let timestamp = current_timestamp();
    let general_firstup = normalize_source_datetime(
        parsed
            .general_firstup
            .clone()
            .or_else(|| parsed.episodes.first().map(|episode| episode.subdate.clone())),
    )
        .or_else(|| existing.and_then(|record| record.general_firstup.clone()));
    let general_lastup = normalize_source_datetime(
        parsed
            .general_lastup
            .clone()
            .or_else(|| {
                parsed
                    .episodes
                    .iter()
                    .rev()
                    .find_map(|episode| Some(episode.subdate.clone()))
            }),
    )
        .or_else(|| existing.and_then(|record| record.general_lastup.clone()));
    let novelupdated_at = normalize_source_datetime(
        parsed
            .novelupdated_at
            .clone()
            .or_else(|| parsed.episodes.iter().rev().find_map(|episode| episode.subupdate.clone()))
            .or_else(|| parsed.episodes.iter().rev().find_map(|episode| Some(episode.subdate.clone()))),
    )
        .or_else(|| existing.and_then(|record| record.novelupdated_at.clone()))
        .or_else(|| general_lastup.clone());
    NovelRecord {
        id: existing.map(|record| record.id).unwrap_or_else(|| {
            database.keys().next_back().map(|id| id + 1).unwrap_or(0)
        }),
        author: parsed.author.clone(),
        title: parsed.title.clone(),
        file_title: file_title.to_string(),
        toc_url,
        sitename: parsed.sitename.clone(),
        novel_type: parsed.novel_type,
        end: parsed.end,
        all_episodes: parsed.episodes.len(),
        last_update: Some(timestamp.clone()),
        new_arrivals_date: existing
            .and_then(|record| record.new_arrivals_date.clone())
            .or(Some(timestamp)),
        use_subdirectory: existing.map(|record| record.use_subdirectory).unwrap_or(false),
        general_firstup,
        novelupdated_at,
        general_lastup,
        length: calculated_length.or(parsed.length).or_else(|| existing.and_then(|record| record.length)),
        suspend: existing.map(|record| record.suspend).unwrap_or(false),
        general_all_no: Some(parsed.episodes.len()),
    }
}

fn build_file_title(existing: Option<&NovelRecord>, parsed: &ParsedNovel, toc_url: &str) -> String {
    if let Some(existing) = existing {
        return existing.file_title.clone();
    }
    let title = replace_filename_special_chars(&parsed.title);
    if let Some(ncode) = extract_ncode(toc_url) {
        truncate_folder_title(&format!("{ncode} {}", title))
    } else {
        truncate_folder_title(&title)
    }
}

fn replace_filename_special_chars(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            _ => ch,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn truncate_folder_title(input: &str) -> String {
    const FOLDER_LENGTH_LIMIT: usize = 50;
    if input.chars().count() <= FOLDER_LENGTH_LIMIT {
        return input.trim().to_string();
    }
    input.chars().take(FOLDER_LENGTH_LIMIT).collect::<String>().trim().to_string()
}

fn extract_ncode(toc_url: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?i)(n\d+[a-z]+)").expect("valid regex");
    re.captures(toc_url)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_ascii_lowercase())
}

fn current_timestamp() -> String {
    format_datetime(Local::now().fixed_offset())
}

fn format_datetime(dt: DateTime<FixedOffset>) -> String {
    format!("{}.{:09} {}", dt.format("%Y-%m-%d %H:%M:%S"), dt.timestamp_subsec_nanos(), dt.format("%:z"))
}

fn parse_source_datetime(input: &str) -> Option<DateTime<FixedOffset>> {
    let trimmed = input.trim();
    let offset = FixedOffset::east_opt(9 * 60 * 60)?;
    for format in [
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d %H:%M",
        "%Y年 %m月%d日 %H時%M分%S秒",
        "%Y年 %m月%d日 %H時%M分",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, format) {
            return offset.from_local_datetime(&naive).single();
        }
    }
    None
}

fn normalize_source_datetime(input: Option<String>) -> Option<String> {
    input.and_then(|value| parse_source_datetime(&value).map(format_datetime))
}

fn changed_episodes(
    old_toc: Option<&Toc>,
    parsed: &ParsedNovel,
    sitename: &str,
    file_title: &str,
    workspace: &Workspace,
) -> Vec<crate::model::Subtitle> {
    match old_toc {
        None => parsed.episodes.clone(),
        Some(old_toc) => parsed
            .episodes
            .iter()
            .filter(|episode| {
                let old = old_toc.subtitles.iter().find(|item| item.index == episode.index);
                match old {
                    None => true,
                    Some(old) => {
                        old != *episode || !workspace.section_exists(sitename, file_title, episode)
                    }
                }
            })
            .cloned()
            .collect(),
    }
}
