mod interactive;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use narou_rust_core::{
    format_error_report, run_doctor, run_repair, App, DoctorOptions, DownloadTarget, ErrorContext,
    RepairOptions,
};
use serde_json::to_string_pretty;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "narou_rust")]
#[command(about = "Fast downloader for Narou-style novels")]
struct Cli {
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    #[arg(long)]
    epub: bool,
    #[arg(long)]
    aozora_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    List {
        #[arg(long)]
        site: Option<String>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long, value_enum, default_value_t = RecordSort::Id)]
        sort: RecordSort,
        #[arg(long)]
        verbose: bool,
        #[arg(long)]
        json: bool,
    },
    Download {
        target: String,
        #[arg(long)]
        no_raw: bool,
    },
    Update {
        targets: Vec<String>,
        #[arg(long)]
        no_raw: bool,
    },
    BatchDownload {
        input: PathBuf,
        #[arg(long)]
        no_raw: bool,
    },
    Convert {
        target: String,
    },
    Inspect {
        target: String,
    },
    Remove {
        target: String,
        #[arg(long)]
        files: bool,
    },
    Doctor {
        ids: Vec<u64>,
        #[arg(long)]
        site: Option<String>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        log_file: Option<PathBuf>,
    },
    Repair {
        ids: Vec<u64>,
        #[arg(long)]
        site: Option<String>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        log_file: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        prune: bool,
    },
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!(
            "{}",
            format_error_report(
                &error,
                &ErrorContext::new().command("cli").workspace(
                    std::env::current_dir()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|_| ".".to_string()),
                ),
            )
        );
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    if std::env::args_os().len() == 1 {
        return interactive::run().await;
    }

    let cli = Cli::parse();
    let workspace_display = cli.workspace.display().to_string();
    let aozora_dir = cli.aozora_dir.clone();

    match cli.command {
        Commands::List {
            site,
            query,
            sort,
            verbose,
            json,
        } => {
            let app = App::new(cli.workspace.clone())?;
            let mut records = app.list_records()?;
            apply_record_filters(&mut records, site.as_deref(), query.as_deref());
            sort_records(&mut records, sort);
            if json {
                println!("{}", to_string_pretty(&records)?);
            } else {
                print_records(&records, verbose);
            }
        }
        Commands::Download { target, no_raw } => {
            let app = App::new(cli.workspace.clone())?;
            let parsed_target = DownloadTarget::parse(&target)?;
            let summary = if cli.epub {
                app.download_with_epub(&parsed_target, !no_raw, aozora_dir.as_deref())
                    .await
            } else {
                app.download(&parsed_target, !no_raw).await
            }
            .with_context(|| {
                format!(
                    "command=download target={} workspace={}",
                    target, workspace_display
                )
            })?;
            println!(
                "downloaded: id={} title={} episodes={}",
                summary.record.id, summary.record.title, summary.episodes_downloaded
            );
            if let Some(epub) = &summary.epub {
                println!("epub: {}", epub.epub_path.display());
            }
        }
        Commands::Update { targets, no_raw } => {
            let app = App::new(cli.workspace.clone())?;
            let targets = if targets.is_empty() {
                Vec::new()
            } else {
                targets
                    .iter()
                    .map(|target| DownloadTarget::parse(target))
                    .collect::<anyhow::Result<Vec<_>>>()?
            };
            let summaries = app.update(targets, !no_raw, aozora_dir.as_deref()).await?;
            for summary in summaries {
                println!(
                    "updated: id={} title={} changed={}",
                    summary.record.id, summary.record.title, summary.episodes_downloaded
                );
                if let Some(epub) = &summary.epub {
                    println!("epub: {}", epub.epub_path.display());
                }
            }
        }
        Commands::BatchDownload { input, no_raw } => {
            let app = App::new(cli.workspace.clone())?;
            let summary = app
                .batch_download(&input, !no_raw, aozora_dir.as_deref())
                .await?;
            for item in &summary.items {
                match &item.result {
                    Ok(download) => {
                        println!(
                            "downloaded: target={} id={} title={} episodes={}",
                            item.target,
                            download.record.id,
                            download.record.title,
                            download.episodes_downloaded
                        );
                        if let Some(epub) = &download.epub {
                            println!("epub: {}", epub.epub_path.display());
                        }
                    }
                    Err(error) => {
                        println!("failed: target={}", item.target);
                        println!("{}", error);
                    }
                }
            }
            println!();
            println!("{}", summary.to_text());
            if summary.failed > 0 {
                std::process::exit(1);
            }
        }
        Commands::Convert { target } => {
            let app = App::new(cli.workspace.clone())?;
            let parsed_target = DownloadTarget::parse(&target)?;
            let summary = app
                .convert_saved(&parsed_target, aozora_dir.as_deref())
                .with_context(|| {
                    format!(
                        "command=convert target={} workspace={}",
                        target, workspace_display
                    )
                })?;
            println!(
                "converted: id={} title={}",
                summary.record.id, summary.record.title
            );
            if let Some(epub) = &summary.epub {
                println!("txt: {}", epub.txt_path.display());
                println!("epub: {}", epub.epub_path.display());
            }
        }
        Commands::Inspect { target } => {
            let app = App::new(cli.workspace.clone())?;
            let parsed_target = DownloadTarget::parse(&target)?;
            let summary = app.inspect(&parsed_target).with_context(|| {
                format!(
                    "command=inspect target={} workspace={}",
                    target, workspace_display
                )
            })?;
            println!("{}", summary.to_text());
        }
        Commands::Remove { target, files } => {
            let app = App::new(cli.workspace.clone())?;
            let parsed_target = DownloadTarget::parse(&target)?;
            let summary = app.remove(&parsed_target, files).with_context(|| {
                format!(
                    "command=remove target={} workspace={}",
                    target, workspace_display
                )
            })?;
            println!(
                "removed: id={} title={} files={}",
                summary.record.id, summary.record.title, summary.removed_files
            );
        }
        Commands::Doctor {
            ids,
            site,
            query,
            json,
            log_file,
        } => {
            let summary = run_doctor(
                cli.workspace.as_path(),
                aozora_dir.as_deref(),
                DoctorOptions { ids, site, query },
            )
            .with_context(|| format!("command=doctor workspace={workspace_display}"))?;
            let output = if json {
                to_string_pretty(&summary)?
            } else {
                summary.to_text()
            };
            print_output(&output);
            if let Some(path) = log_file {
                write_log_file(&path, &output)?;
            }
            let exit_code = summary.exit_code();
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
        }
        Commands::Repair {
            ids,
            site,
            query,
            json,
            log_file,
            dry_run,
            prune,
        } => {
            let summary = run_repair(
                cli.workspace.as_path(),
                aozora_dir.as_deref(),
                RepairOptions {
                    dry_run,
                    prune,
                    ids,
                    site,
                    query,
                },
            )
            .await
            .with_context(|| format!("command=repair workspace={workspace_display}"))?;
            let output = if json {
                to_string_pretty(&summary)?
            } else {
                summary.to_text()
            };
            print_output(&output);
            if let Some(path) = log_file {
                write_log_file(&path, &output)?;
            }
            let exit_code = summary.exit_code();
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
        }
    }

    Ok(())
}

fn print_output(output: &str) {
    println!("{output}");
}

fn write_log_file(path: &std::path::Path, output: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, output)?;
    Ok(())
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum RecordSort {
    Id,
    Title,
    Site,
    Author,
    Episodes,
}

fn apply_record_filters(
    records: &mut Vec<narou_rust_core::NovelRecord>,
    site: Option<&str>,
    query: Option<&str>,
) {
    if let Some(site) = site {
        let site = site.trim().to_ascii_lowercase();
        records.retain(|record| record.sitename.to_ascii_lowercase().contains(&site));
    }
    if let Some(query) = query {
        let query = query.trim().to_ascii_lowercase();
        records.retain(|record| {
            record.title.to_ascii_lowercase().contains(&query)
                || record.author.to_ascii_lowercase().contains(&query)
                || record.toc_url.to_ascii_lowercase().contains(&query)
        });
    }
}

fn sort_records(records: &mut [narou_rust_core::NovelRecord], sort: RecordSort) {
    match sort {
        RecordSort::Id => records.sort_by_key(|record| record.id),
        RecordSort::Title => {
            records.sort_by_cached_key(|record| (record.title.to_ascii_lowercase(), record.id))
        }
        RecordSort::Site => records.sort_by_cached_key(|record| {
            (
                record.sitename.to_ascii_lowercase(),
                record.title.to_ascii_lowercase(),
                record.id,
            )
        }),
        RecordSort::Author => records.sort_by_cached_key(|record| {
            (
                record.author.to_ascii_lowercase(),
                record.title.to_ascii_lowercase(),
                record.id,
            )
        }),
        RecordSort::Episodes => records.sort_by_cached_key(|record| {
            (
                std::cmp::Reverse(record.all_episodes),
                record.title.to_ascii_lowercase(),
                record.id,
            )
        }),
    }
}

fn print_records(records: &[narou_rust_core::NovelRecord], verbose: bool) {
    if records.is_empty() {
        println!("保存済み作品はありません。");
        return;
    }
    for record in records {
        if verbose {
            println!(
                "[{}] {} | {} | {} | episodes={} | url={}",
                record.id,
                record.sitename,
                record.title,
                record.author,
                record.all_episodes,
                record.toc_url
            );
        } else {
            println!(
                "[{}] {} | {} | {} | episodes={}",
                record.id, record.sitename, record.title, record.author, record.all_episodes
            );
        }
    }
    println!("total: {}", records.len());
}
