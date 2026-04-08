use crate::config::{global_setting_path, local_setting_path, AppConfig};
use crate::convert::resolve_aozora_dir;
use crate::model::NovelRecord;
use crate::storage::Workspace;
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

const ARCHIVE_ROOT: &str = "小説データ";
const SECTION_DIR: &str = "本文";
const RAW_DIR: &str = "raw";

#[derive(Debug, Clone, Serialize)]
pub struct DoctorSummary {
    pub workspace: String,
    pub workspace_exists: bool,
    pub narou_dir: String,
    pub narou_dir_exists: bool,
    pub database_path: String,
    pub database_exists: bool,
    pub local_setting_path: String,
    pub local_setting_exists: bool,
    pub global_setting_path: String,
    pub global_setting_exists: bool,
    pub saved_records: usize,
    pub config: DoctorConfigSummary,
    pub java: DoctorCheck,
    pub aozora: DoctorAozoraSummary,
    pub archive: DoctorArchiveSummary,
    pub issues: Vec<DoctorIssue>,
    pub records: Vec<DoctorRecordHealth>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DoctorOptions {
    pub ids: Vec<u64>,
    pub site: Option<String>,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorConfigSummary {
    pub download_interval_secs: f64,
    pub update_interval_secs: f64,
    pub download_wait_steps: u64,
    pub retry_limit: usize,
    pub retry_wait_secs: f64,
    pub long_wait_secs: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorAozoraSummary {
    pub requested_path: Option<String>,
    pub resolved_path: Option<String>,
    pub jar_path: Option<String>,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorArchiveSummary {
    pub archive_root: String,
    pub archive_root_exists: bool,
    pub orphan_dirs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorIssue {
    pub level: String,
    pub code: String,
    pub summary: String,
    pub record_id: Option<u64>,
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorRecordHealth {
    pub id: u64,
    pub site: String,
    pub title: String,
    pub novel_dir: String,
    pub novel_dir_exists: bool,
    pub toc_exists: bool,
    pub expected_episodes: usize,
    pub section_files: usize,
    pub raw_files: usize,
    pub txt_exists: bool,
    pub epub_files: usize,
    pub ok: bool,
    pub needs_update: bool,
    pub needs_convert: bool,
    pub suggestions: Vec<String>,
}

pub fn run_doctor(
    workspace_root: &Path,
    explicit_aozora_dir: Option<&Path>,
    options: DoctorOptions,
) -> Result<DoctorSummary> {
    let workspace_root = workspace_root.to_path_buf();
    let workspace = Workspace::new(workspace_root.clone())?;
    let config = AppConfig::load(&workspace_root)?;
    let narou_dir = workspace_root.join(".narou");
    let database_path = narou_dir.join("database.yaml");
    let local_path = local_setting_path(&workspace_root);
    let global_path = global_setting_path();
    let database = workspace.load_database()?;
    let java = detect_java();
    let aozora = detect_aozora(&workspace_root, explicit_aozora_dir);
    let (archive, mut issues) = inspect_archive(&workspace_root, &database)?;
    append_environment_issues(&mut issues, &java, &aozora, database_path.exists());
    let mut records = inspect_records(&workspace, &database, aozora.ok, &mut issues);
    apply_record_filters(&mut records, &options);
    issues = filter_issues(&issues, &records, &archive, &options);

    Ok(DoctorSummary {
        workspace: workspace_root.display().to_string(),
        workspace_exists: workspace_root.exists(),
        narou_dir: narou_dir.display().to_string(),
        narou_dir_exists: narou_dir.exists(),
        database_path: database_path.display().to_string(),
        database_exists: database_path.exists(),
        local_setting_path: local_path.display().to_string(),
        local_setting_exists: local_path.exists(),
        global_setting_path: global_path.display().to_string(),
        global_setting_exists: global_path.exists(),
        saved_records: database.len(),
        config: DoctorConfigSummary {
            download_interval_secs: config.download_interval_secs,
            update_interval_secs: config.update_interval_secs,
            download_wait_steps: config.download_wait_steps,
            retry_limit: config.retry_limit,
            retry_wait_secs: config.retry_wait_secs,
            long_wait_secs: config.long_wait_secs,
        },
        java,
        aozora,
        archive,
        issues,
        records,
    })
}

fn filter_issues(
    issues: &[DoctorIssue],
    records: &[DoctorRecordHealth],
    archive: &DoctorArchiveSummary,
    options: &DoctorOptions,
) -> Vec<DoctorIssue> {
    let record_ids = records.iter().map(|record| record.id).collect::<Vec<_>>();
    let keep_archive = archive
        .orphan_dirs
        .iter()
        .any(|path| orphan_matches(path, options));
    issues
        .iter()
        .filter(|issue| match issue.record_id {
            Some(id) => record_ids.contains(&id),
            None => {
                if issue.code.starts_with("archive.") {
                    keep_archive
                } else {
                    true
                }
            }
        })
        .cloned()
        .collect()
}

fn apply_record_filters(records: &mut Vec<DoctorRecordHealth>, options: &DoctorOptions) {
    if !options.ids.is_empty() {
        records.retain(|record| options.ids.contains(&record.id));
    }
    if let Some(site) = options.site.as_deref() {
        let site = site.trim().to_ascii_lowercase();
        if !site.is_empty() {
            records.retain(|record| record.site.to_ascii_lowercase().contains(&site));
        }
    }
    if let Some(query) = options.query.as_deref() {
        let query = query.trim().to_ascii_lowercase();
        if !query.is_empty() {
            records.retain(|record| {
                record.title.to_ascii_lowercase().contains(&query)
                    || record.site.to_ascii_lowercase().contains(&query)
                    || record.novel_dir.to_ascii_lowercase().contains(&query)
            });
        }
    }
}

fn orphan_matches(path: &str, options: &DoctorOptions) -> bool {
    let path_lower = path.to_ascii_lowercase();
    if let Some(site) = options.site.as_deref() {
        let site = site.trim().to_ascii_lowercase();
        if !site.is_empty() && !path_lower.contains(&site) {
            return false;
        }
    }
    if let Some(query) = options.query.as_deref() {
        let query = query.trim().to_ascii_lowercase();
        if !query.is_empty() && !path_lower.contains(&query) {
            return false;
        }
    }
    true
}

fn append_environment_issues(
    issues: &mut Vec<DoctorIssue>,
    java: &DoctorCheck,
    aozora: &DoctorAozoraSummary,
    database_exists: bool,
) {
    if !database_exists {
        issues.push(DoctorIssue {
            level: String::from("warning"),
            code: String::from("workspace.database_missing"),
            summary: String::from("database.yaml がまだありません"),
            record_id: None,
            suggestions: vec![
                String::from("最初の作品を download すると自動で作成されます"),
                String::from("既存データがあるはずなら workspace の指定先を見直す"),
            ],
        });
    }

    if !java.ok {
        issues.push(DoctorIssue {
            level: String::from("error"),
            code: String::from("environment.java_missing"),
            summary: format!("Java が利用できません: {}", java.detail),
            record_id: None,
            suggestions: vec![
                String::from("Java をインストールする"),
                String::from("新しいターミナルで PATH を再読み込みする"),
            ],
        });
    }

    if !aozora.ok {
        issues.push(DoctorIssue {
            level: String::from("warning"),
            code: String::from("environment.aozora_unavailable"),
            summary: aozora.detail.clone(),
            record_id: None,
            suggestions: vec![
                String::from("AozoraEpub3-1.1.1b30Q を workspace または親ディレクトリに置く"),
                String::from("--aozora-dir で AozoraEpub3 の場所を指定する"),
            ],
        });
    }
}

fn inspect_records(
    workspace: &Workspace,
    database: &BTreeMap<u64, NovelRecord>,
    aozora_ok: bool,
    issues: &mut Vec<DoctorIssue>,
) -> Vec<DoctorRecordHealth> {
    let mut records = Vec::new();
    for record in database.values() {
        let novel_dir = workspace.novel_dir(&record.sitename, &record.file_title);
        let toc_path = novel_dir.join("toc.yaml");
        let section_dir = novel_dir.join(SECTION_DIR);
        let raw_dir = novel_dir.join(RAW_DIR);
        let txt_path = novel_dir.join(format!("{}.txt", record.file_title));
        let epub_files = count_files(&novel_dir, "epub");
        let toc_exists = toc_path.exists();
        let toc = if toc_exists {
            workspace.load_toc(&record.sitename, &record.file_title).ok()
        } else {
            None
        };
        let expected_episodes = toc
            .as_ref()
            .map(|toc| toc.subtitles.len())
            .unwrap_or(record.all_episodes);
        let section_files = count_files(&section_dir, "yaml");
        let raw_files = count_files(&raw_dir, "html");
        let txt_exists = txt_path.exists();

        let mut suggestions = Vec::new();
        let mut ok = true;
        let mut needs_update = false;

        if !novel_dir.exists() {
            ok = false;
            needs_update = true;
            issues.push(DoctorIssue {
                level: String::from("error"),
                code: String::from("record.missing_novel_dir"),
                summary: format!("id={} title={} の作品フォルダがありません", record.id, record.title),
                record_id: Some(record.id),
                suggestions: vec![
                    format!("narou_rust update {}", record.id),
                    format!("不要なら narou_rust remove {}", record.id),
                ],
            });
        }

        if !toc_exists {
            ok = false;
            needs_update = true;
            issues.push(DoctorIssue {
                level: String::from("error"),
                code: String::from("record.missing_toc"),
                summary: format!("id={} title={} の toc.yaml がありません", record.id, record.title),
                record_id: Some(record.id),
                suggestions: vec![format!("narou_rust update {}", record.id)],
            });
            suggestions.push(format!("narou_rust update {}", record.id));
        }

        if expected_episodes > 0 && section_files < expected_episodes {
            ok = false;
            needs_update = true;
            issues.push(DoctorIssue {
                level: String::from("error"),
                code: String::from("record.missing_sections"),
                summary: format!(
                    "id={} title={} の本文ファイルが不足しています ({}/{})",
                    record.id, record.title, section_files, expected_episodes
                ),
                record_id: Some(record.id),
                suggestions: vec![format!("narou_rust update {}", record.id)],
            });
            push_unique(&mut suggestions, format!("narou_rust update {}", record.id));
        }

        if let Some(toc) = &toc {
            if toc.title != record.title || toc.author != record.author || toc.toc_url != record.toc_url {
                needs_update = true;
                issues.push(DoctorIssue {
                    level: String::from("warning"),
                    code: String::from("record.metadata_mismatch"),
                    summary: format!("id={} title={} の database.yaml と toc.yaml のメタデータが一致していません", record.id, record.title),
                    record_id: Some(record.id),
                    suggestions: vec![format!("narou_rust update {}", record.id)],
                });
                push_unique(&mut suggestions, format!("narou_rust update {}", record.id));
            }
        }

        let needs_convert = (!txt_exists || epub_files == 0) && aozora_ok && novel_dir.exists() && toc_exists;
        if !txt_exists {
            if aozora_ok {
                suggestions.push(format!("narou_rust convert {}", record.id));
            }
        } else if epub_files == 0 && aozora_ok {
            suggestions.push(format!("narou_rust convert {}", record.id));
        }

        records.push(DoctorRecordHealth {
            id: record.id,
            site: record.sitename.clone(),
            title: record.title.clone(),
            novel_dir: novel_dir.display().to_string(),
            novel_dir_exists: novel_dir.exists(),
            toc_exists,
            expected_episodes,
            section_files,
            raw_files,
            txt_exists,
            epub_files,
            ok,
            needs_update,
            needs_convert,
            suggestions,
        });
    }
    records
}

fn inspect_archive(
    workspace_root: &Path,
    database: &BTreeMap<u64, NovelRecord>,
) -> Result<(DoctorArchiveSummary, Vec<DoctorIssue>)> {
    let archive_root = workspace_root.join(ARCHIVE_ROOT);
    let mut orphan_dirs = Vec::new();
    let mut issues = Vec::new();
    let known_dirs = database
        .values()
        .map(|record| archive_root.join(&record.sitename).join(&record.file_title))
        .collect::<Vec<_>>();

    if archive_root.exists() {
        for site_entry in fs::read_dir(&archive_root)? {
            let site_entry = site_entry?;
            let site_path = site_entry.path();
            if !site_path.is_dir() {
                continue;
            }
            for novel_entry in fs::read_dir(site_path)? {
                let novel_entry = novel_entry?;
                let novel_path = novel_entry.path();
                if !novel_path.is_dir() {
                    continue;
                }
                if !known_dirs.iter().any(|known| known == &novel_path) {
                    let path_text = novel_path.display().to_string();
                    orphan_dirs.push(path_text.clone());
                    issues.push(DoctorIssue {
                        level: String::from("warning"),
                        code: String::from("archive.orphan_dir"),
                        summary: format!("database.yaml に存在しない作品フォルダがあります: {path_text}"),
                        record_id: None,
                        suggestions: vec![
                            String::from("必要なら database.yaml へ再登録するために download/update を実行する"),
                            format!("不要ならディレクトリを削除する: {}", path_text),
                        ],
                    });
                }
            }
        }
    }

    Ok((
        DoctorArchiveSummary {
            archive_root: archive_root.display().to_string(),
            archive_root_exists: archive_root.exists(),
            orphan_dirs,
        },
        issues,
    ))
}

fn detect_java() -> DoctorCheck {
    match Command::new("java").arg("-version").output() {
        Ok(output) => {
            let detail = if output.stderr.is_empty() {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("java found")
                    .to_string()
            } else {
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .next()
                    .unwrap_or("java found")
                    .to_string()
            };
            DoctorCheck {
                ok: output.status.success(),
                detail,
            }
        }
        Err(error) => DoctorCheck {
            ok: false,
            detail: format!("java not available: {error}"),
        },
    }
}

fn detect_aozora(workspace_root: &Path, explicit_aozora_dir: Option<&Path>) -> DoctorAozoraSummary {
    let requested_path = explicit_aozora_dir.map(display_path);
    match resolve_aozora_dir(workspace_root, explicit_aozora_dir) {
        Ok(path) => {
            let jar_path = path.join("AozoraEpub3.jar");
            let ok = jar_path.exists();
            DoctorAozoraSummary {
                requested_path,
                resolved_path: Some(path.display().to_string()),
                jar_path: Some(jar_path.display().to_string()),
                ok,
                detail: if ok {
                    String::from("AozoraEpub3.jar found")
                } else {
                    format!("AozoraEpub3.jar not found: {}", jar_path.display())
                },
            }
        }
        Err(error) => DoctorAozoraSummary {
            requested_path,
            resolved_path: None,
            jar_path: None,
            ok: false,
            detail: error.to_string(),
        },
    }
}

fn count_files(dir: &Path, extension: &str) -> usize {
    if !dir.exists() {
        return 0;
    }
    fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case(extension))
                == Some(true)
        })
        .count()
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|item| item == &value) {
        values.push(value);
    }
}

impl DoctorSummary {
    pub fn error_count(&self) -> usize {
        self.issues.iter().filter(|issue| issue.level == "error").count()
    }

    pub fn warning_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.level == "warning")
            .count()
    }

    pub fn exit_code(&self) -> i32 {
        if self.error_count() > 0 {
            3
        } else if self.warning_count() > 0 {
            2
        } else {
            0
        }
    }

    pub fn to_text(&self) -> String {
        let error_count = self.error_count();
        let warning_count = self.warning_count();

        let mut lines = vec![
            format!("workspace: {}", self.workspace),
            format!("workspace_exists: {}", self.workspace_exists),
            format!("narou_dir: {}", self.narou_dir),
            format!("narou_dir_exists: {}", self.narou_dir_exists),
            format!("database_path: {}", self.database_path),
            format!("database_exists: {}", self.database_exists),
            format!("local_setting_path: {}", self.local_setting_path),
            format!("local_setting_exists: {}", self.local_setting_exists),
            format!("global_setting_path: {}", self.global_setting_path),
            format!("global_setting_exists: {}", self.global_setting_exists),
            format!("saved_records: {}", self.saved_records),
            String::from("config:"),
            format!("  download_interval_secs: {}", self.config.download_interval_secs),
            format!("  update_interval_secs: {}", self.config.update_interval_secs),
            format!("  download_wait_steps: {}", self.config.download_wait_steps),
            format!("  retry_limit: {}", self.config.retry_limit),
            format!("  retry_wait_secs: {}", self.config.retry_wait_secs),
            format!("  long_wait_secs: {}", self.config.long_wait_secs),
            format!("java_ok: {}", self.java.ok),
            format!("java_detail: {}", self.java.detail),
            format!(
                "aozora_requested_path: {}",
                self.aozora.requested_path.as_deref().unwrap_or("(auto)")
            ),
            format!(
                "aozora_resolved_path: {}",
                self.aozora.resolved_path.as_deref().unwrap_or("(not found)")
            ),
            format!(
                "aozora_jar_path: {}",
                self.aozora.jar_path.as_deref().unwrap_or("(not found)")
            ),
            format!("aozora_ok: {}", self.aozora.ok),
            format!("aozora_detail: {}", self.aozora.detail),
            format!("archive_root: {}", self.archive.archive_root),
            format!("archive_root_exists: {}", self.archive.archive_root_exists),
            format!("orphan_dirs: {}", self.archive.orphan_dirs.len()),
            format!("issues: total={} error={} warning={}", self.issues.len(), error_count, warning_count),
        ];

        if !self.issues.is_empty() {
            lines.push(String::from("issue_details:"));
            for issue in &self.issues {
                lines.push(format!(
                    "- [{}] {}: {}",
                    issue.level, issue.code, issue.summary
                ));
                for suggestion in &issue.suggestions {
                    lines.push(format!("  suggestion: {}", suggestion));
                }
            }
        }

        if !self.records.is_empty() {
            lines.push(String::from("record_health:"));
            for record in &self.records {
                lines.push(format!(
                    "- id={} ok={} site={} title={} toc={} sections={}/{} raw={} txt={} epub={} needs_update={} needs_convert={}",
                    record.id,
                    record.ok,
                    record.site,
                    record.title,
                    record.toc_exists,
                    record.section_files,
                    record.expected_episodes,
                    record.raw_files,
                    record.txt_exists,
                    record.epub_files,
                    record.needs_update,
                    record.needs_convert
                ));
                for suggestion in &record.suggestions {
                    lines.push(format!("  suggestion: {}", suggestion));
                }
            }
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DoctorArchiveSummary, DoctorAozoraSummary, DoctorCheck, DoctorConfigSummary, DoctorIssue,
        DoctorRecordHealth, DoctorSummary,
    };

    #[test]
    fn renders_doctor_summary_text() {
        let summary = DoctorSummary {
            workspace: String::from("D:\\work"),
            workspace_exists: true,
            narou_dir: String::from("D:\\work\\.narou"),
            narou_dir_exists: true,
            database_path: String::from("D:\\work\\.narou\\database.yaml"),
            database_exists: true,
            local_setting_path: String::from("D:\\work\\.narou\\local_setting.yaml"),
            local_setting_exists: false,
            global_setting_path: String::from("C:\\Users\\user\\.narousetting\\global_setting.yaml"),
            global_setting_exists: true,
            saved_records: 3,
            config: DoctorConfigSummary {
                download_interval_secs: 0.7,
                update_interval_secs: 0.0,
                download_wait_steps: 10,
                retry_limit: 5,
                retry_wait_secs: 10.0,
                long_wait_secs: 5.0,
            },
            java: DoctorCheck {
                ok: true,
                detail: String::from("openjdk version \"21\""),
            },
            aozora: DoctorAozoraSummary {
                requested_path: None,
                resolved_path: Some(String::from("D:\\AozoraEpub3-1.1.1b30Q")),
                jar_path: Some(String::from("D:\\AozoraEpub3-1.1.1b30Q\\AozoraEpub3.jar")),
                ok: true,
                detail: String::from("AozoraEpub3.jar found"),
            },
            archive: DoctorArchiveSummary {
                archive_root: String::from("D:\\work\\小説データ"),
                archive_root_exists: true,
                orphan_dirs: vec![String::from("D:\\work\\小説データ\\site\\orphan")],
            },
            issues: vec![DoctorIssue {
                level: String::from("warning"),
                code: String::from("archive.orphan_dir"),
                summary: String::from("orphan found"),
                record_id: None,
                suggestions: vec![String::from("remove orphan dir")],
            }],
            records: vec![DoctorRecordHealth {
                id: 0,
                site: String::from("小説家になろう"),
                title: String::from("作品"),
                novel_dir: String::from("D:\\work\\小説データ\\小説家になろう\\作品"),
                novel_dir_exists: true,
                toc_exists: true,
                expected_episodes: 10,
                section_files: 9,
                raw_files: 0,
                txt_exists: false,
                epub_files: 0,
                ok: false,
                needs_update: true,
                needs_convert: true,
                suggestions: vec![String::from("narou_rust update 0")],
            }],
        };

        let text = summary.to_text();
        assert!(text.contains("issues: total=1 error=0 warning=1"));
        assert!(text.contains("archive.orphan_dir"));
        assert!(text.contains("record_health:"));
        assert!(text.contains("suggestion: narou_rust update 0"));
    }
}
