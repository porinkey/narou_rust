use crate::app::App;
use crate::doctor::{run_doctor, DoctorIssue, DoctorOptions};
use crate::model::DownloadTarget;
use anyhow::Result;
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct RepairOptions {
    pub dry_run: bool,
    pub prune: bool,
    pub ids: Vec<u64>,
    pub site: Option<String>,
    pub query: Option<String>,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            prune: false,
            ids: Vec::new(),
            site: None,
            query: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairSummary {
    pub workspace: String,
    pub dry_run: bool,
    pub prune: bool,
    pub target_ids: Vec<u64>,
    pub planned_update_ids: Vec<u64>,
    pub planned_convert_ids: Vec<u64>,
    pub updated_ids: Vec<u64>,
    pub converted_ids: Vec<u64>,
    pub pruned_dirs: Vec<String>,
    pub skipped_ids: Vec<u64>,
    pub unresolved_issues: Vec<DoctorIssue>,
}

pub async fn run_repair(
    workspace_root: &Path,
    explicit_aozora_dir: Option<&Path>,
    options: RepairOptions,
) -> Result<RepairSummary> {
    let before = run_doctor(
        workspace_root,
        explicit_aozora_dir,
        DoctorOptions {
            ids: options.ids.clone(),
            site: options.site.clone(),
            query: options.query.clone(),
        },
    )?;
    let target_ids = options.ids.clone();
    let selected_records = select_records(&before.records, &target_ids);
    let planned_update_ids = selected_records
        .records
        .iter()
        .filter(|record| record.needs_update)
        .map(|record| record.id)
        .collect::<Vec<_>>();
    let mut planned_convert_ids = before
        .records
        .iter()
        .filter(|record| selected_records.ids.contains(&record.id))
        .filter(|record| !record.needs_update && record.needs_convert)
        .map(|record| record.id)
        .collect::<Vec<_>>();
    let mut updated_ids = Vec::new();
    let mut converted_ids = Vec::new();
    let mut pruned_dirs = Vec::new();
    let mut skipped_ids = Vec::new();

    if options.dry_run {
        if options.prune {
            pruned_dirs = before.archive.orphan_dirs.clone();
        }
        return Ok(RepairSummary {
            workspace: workspace_root.display().to_string(),
            dry_run: true,
            prune: options.prune,
            target_ids,
            planned_update_ids,
            planned_convert_ids,
            updated_ids,
            converted_ids,
            pruned_dirs,
            skipped_ids,
            unresolved_issues: filter_issues(&before.issues, &selected_records.ids, options.prune),
        });
    }

    if options.prune {
        pruned_dirs = prune_orphan_dirs(workspace_root, &before.archive.orphan_dirs)?;
    }

    let app = App::new(workspace_root.to_path_buf())?;

    if !planned_update_ids.is_empty() {
        let targets = planned_update_ids
            .iter()
            .copied()
            .map(DownloadTarget::Id)
            .collect::<Vec<_>>();
        let summaries = app.update(targets, true, None).await?;
        updated_ids = summaries.into_iter().map(|summary| summary.record.id).collect();
    }

    let after_update = run_doctor(
        workspace_root,
        explicit_aozora_dir,
        DoctorOptions {
            ids: options.ids.clone(),
            site: options.site.clone(),
            query: options.query.clone(),
        },
    )?;
    planned_convert_ids = after_update
        .records
        .iter()
        .filter(|record| selected_records.ids.contains(&record.id))
        .filter(|record| record.needs_convert)
        .map(|record| record.id)
        .collect::<Vec<_>>();

    for id in planned_convert_ids.iter().copied() {
        match app.convert_saved(&DownloadTarget::Id(id), explicit_aozora_dir) {
            Ok(_) => converted_ids.push(id),
            Err(_) => skipped_ids.push(id),
        }
    }

    let after = run_doctor(
        workspace_root,
        explicit_aozora_dir,
        DoctorOptions {
            ids: options.ids.clone(),
            site: options.site.clone(),
            query: options.query.clone(),
        },
    )?;
    Ok(RepairSummary {
        workspace: workspace_root.display().to_string(),
        dry_run: false,
        prune: options.prune,
        target_ids,
        planned_update_ids,
        planned_convert_ids,
        updated_ids,
        converted_ids,
        pruned_dirs,
        skipped_ids,
        unresolved_issues: filter_issues(&after.issues, &selected_records.ids, options.prune),
    })
}

impl RepairSummary {
    pub fn error_count(&self) -> usize {
        self.unresolved_issues
            .iter()
            .filter(|issue| issue.level == "error")
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.unresolved_issues
            .iter()
            .filter(|issue| issue.level == "warning")
            .count()
    }

    pub fn exit_code(&self) -> i32 {
        if self.error_count() > 0 {
            3
        } else if self.warning_count() > 0 || !self.skipped_ids.is_empty() {
            2
        } else {
            0
        }
    }

    pub fn to_text(&self) -> String {
        let mut lines = vec![
            format!("workspace: {}", self.workspace),
            format!("dry_run: {}", self.dry_run),
            format!("prune: {}", self.prune),
            format!("target_ids: {}", render_id_list(&self.target_ids)),
            format!("planned_updates: {}", render_id_list(&self.planned_update_ids)),
            format!("planned_converts: {}", render_id_list(&self.planned_convert_ids)),
            format!("updated: {}", render_id_list(&self.updated_ids)),
            format!("converted: {}", render_id_list(&self.converted_ids)),
            format!("pruned_dirs: {}", render_path_list(&self.pruned_dirs)),
            format!("skipped: {}", render_id_list(&self.skipped_ids)),
            format!(
                "unresolved_issues: total={} error={} warning={}",
                self.unresolved_issues.len(),
                self.error_count(),
                self.warning_count()
            ),
        ];
        if !self.unresolved_issues.is_empty() {
            lines.push(String::from("issue_details:"));
            for issue in &self.unresolved_issues {
                lines.push(format!(
                    "- [{}] {}: {}",
                    issue.level, issue.code, issue.summary
                ));
            }
        }
        lines.join("\n")
    }
}

struct SelectedRecords {
    ids: Vec<u64>,
    records: Vec<crate::doctor::DoctorRecordHealth>,
}

fn select_records(
    records: &[crate::doctor::DoctorRecordHealth],
    target_ids: &[u64],
) -> SelectedRecords {
    if target_ids.is_empty() {
        return SelectedRecords {
            ids: records.iter().map(|record| record.id).collect(),
            records: records.to_vec(),
        };
    }
    let selected = records
        .iter()
        .filter(|record| target_ids.contains(&record.id))
        .cloned()
        .collect::<Vec<_>>();
    SelectedRecords {
        ids: selected.iter().map(|record| record.id).collect(),
        records: selected,
    }
}

fn filter_issues(issues: &[DoctorIssue], target_ids: &[u64], include_prune: bool) -> Vec<DoctorIssue> {
    issues
        .iter()
        .filter(|issue| match issue.record_id {
            Some(id) => target_ids.is_empty() || target_ids.contains(&id),
            None => include_prune || !issue.code.starts_with("archive."),
        })
        .cloned()
        .collect()
}

fn render_id_list(ids: &[u64]) -> String {
    if ids.is_empty() {
        String::from("(none)")
    } else {
        ids.iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn render_path_list(paths: &[String]) -> String {
    if paths.is_empty() {
        String::from("(none)")
    } else {
        paths.join(", ")
    }
}

fn prune_orphan_dirs(workspace_root: &Path, orphan_dirs: &[String]) -> Result<Vec<String>> {
    let archive_root = workspace_root.join("小説データ");
    let mut removed = Vec::new();
    for orphan in orphan_dirs {
        let path = Path::new(orphan);
        if path.exists() && path.starts_with(&archive_root) {
            fs::remove_dir_all(path)?;
            removed.push(orphan.clone());
        }
    }
    Ok(removed)
}
