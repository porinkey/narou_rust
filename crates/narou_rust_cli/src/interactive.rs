use anyhow::{bail, Context, Result};
use narou_rust_core::{run_doctor, run_repair, App, DoctorOptions, DownloadTarget, RepairOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct InteractiveSession {
    pub workspace: PathBuf,
    pub epub: bool,
    pub aozora_dir: Option<PathBuf>,
}

pub async fn run() -> Result<()> {
    println!("narou_rust CLI menu");
    println!();

    loop {
        println!("1. ダウンロード");
        println!("2. 更新");
        println!("3. 一括ダウンロード");
        println!("4. 保存済み作品一覧");
        println!("5. 保存済み作品の再変換");
        println!("6. 保存済み作品の詳細表示");
        println!("7. 保存済み作品の削除");
        println!("8. doctor");
        println!("9. repair");
        println!("10. 終了");
        println!();

        match prompt_menu("メニューを選んでください", 10)? {
            1 => run_download_menu().await?,
            2 => run_update_menu().await?,
            3 => run_batch_download_menu().await?,
            4 => run_list_menu()?,
            5 => run_convert_menu()?,
            6 => run_inspect_menu()?,
            7 => run_remove_menu()?,
            8 => run_doctor_menu()?,
            9 => run_repair_menu().await?,
            10 => break,
            _ => unreachable!(),
        }

        println!();
    }

    Ok(())
}

async fn run_download_menu() -> Result<()> {
    println!();
    println!("ダウンロード");
    println!("1. 小説家になろう");
    println!("2. R18系");
    println!("3. カクヨム");
    println!();

    let site = prompt_menu("サイトを選んでください", 3)?;
    let session = prompt_common_options()?;
    let target = match site {
        1 => prompt_non_empty("URL または Nコードを入力してください")?,
        2 => prompt_non_empty("R18系の作品URLを入力してください")?,
        3 => prompt_non_empty("カクヨムの作品URLを入力してください")?,
        _ => unreachable!(),
    };

    let app = App::new(session.workspace.clone())?;
    let parsed_target = DownloadTarget::parse(&target)?;
    let summary = if session.epub {
        app.download_with_epub(&parsed_target, true, session.aozora_dir.as_deref())
            .await
    } else {
        app.download(&parsed_target, true).await
    }
    .with_context(|| {
        format!(
            "command=download target={} workspace={}",
            target,
            session.workspace.display()
        )
    })?;

    println!(
        "downloaded: id={} title={} episodes={}",
        summary.record.id, summary.record.title, summary.episodes_downloaded
    );
    if let Some(epub) = &summary.epub {
        println!("epub: {}", epub.epub_path.display());
    }
    Ok(())
}

async fn run_update_menu() -> Result<()> {
    println!();
    println!("更新");
    println!("1. 保存済み作品をすべて更新");
    println!("2. 対象を指定して更新");
    println!("3. 保存済み作品から選んで更新");
    println!();

    let mode = prompt_menu("更新方法を選んでください", 3)?;
    let session = prompt_common_options()?;
    let app = App::new(session.workspace.clone())?;
    let targets = if mode == 1 {
        Vec::new()
    } else if mode == 2 {
        let raw = prompt_non_empty("URL または Nコードを空白区切りで入力してください")?;
        raw.split_whitespace()
            .map(DownloadTarget::parse)
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        let records = app.list_records()?;
        if records.is_empty() {
            println!("保存済み作品はありません。");
            return Ok(());
        }
        print_records(&records, false);
        let raw = prompt_non_empty("更新したい id を空白区切りで入力してください")?;
        let ids = parse_id_list(&raw)?;
        ids.into_iter().map(DownloadTarget::Id).collect()
    };

    let summaries = app
        .update(targets, true, session.aozora_dir.as_deref())
        .await?;
    for summary in summaries {
        println!(
            "updated: id={} title={} changed={}",
            summary.record.id, summary.record.title, summary.episodes_downloaded
        );
        if let Some(epub) = &summary.epub {
            println!("epub: {}", epub.epub_path.display());
        }
    }
    Ok(())
}

async fn run_batch_download_menu() -> Result<()> {
    println!();
    println!("一括ダウンロード");
    let session = prompt_common_options()?;
    let input = PathBuf::from(prompt_non_empty("URL一覧ファイルのパスを入力してください")?);
    if !input.exists() {
        bail!("input file not found: {}", input.display());
    }

    let app = App::new(session.workspace.clone())?;
    let summary = app
        .batch_download(&input, true, session.aozora_dir.as_deref())
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
    Ok(())
}

fn run_list_menu() -> Result<()> {
    println!();
    println!("保存済み作品一覧");
    let default_workspace = Path::new(".").display().to_string();
    let workspace_input =
        prompt_with_default("workspace を入力してください", &default_workspace)?;
    let app = App::new(PathBuf::from(workspace_input))?;
    let mut records = app.list_records()?;
    let site = prompt_optional("サイト名で絞り込みますか？（未入力で全件）")?;
    let query = prompt_optional("タイトル/作者/URLで検索しますか？（未入力で全件）")?;
    let sort = prompt_sort()?;
    let verbose = prompt_yes_no("URL も表示しますか？", false)?;
    apply_record_filters(&mut records, site.as_deref(), query.as_deref());
    sort_records(&mut records, sort);
    if records.is_empty() {
        println!("保存済み作品はありません。");
    } else {
        print_records(&records, verbose);
    }
    Ok(())
}

fn run_convert_menu() -> Result<()> {
    println!();
    println!("保存済み作品の再変換");
    let session = prompt_common_options()?;
    let app = App::new(session.workspace.clone())?;
    let records = app.list_records()?;
    if records.is_empty() {
        println!("保存済み作品はありません。");
        return Ok(());
    }
    print_records(&records, false);
    let id = prompt_single_id("再変換したい id を入力してください")?;
    let summary = app.convert_saved(&DownloadTarget::Id(id), session.aozora_dir.as_deref())?;
    println!(
        "converted: id={} title={}",
        summary.record.id, summary.record.title
    );
    if let Some(epub) = &summary.epub {
        println!("txt: {}", epub.txt_path.display());
        println!("epub: {}", epub.epub_path.display());
    }
    Ok(())
}

fn run_inspect_menu() -> Result<()> {
    println!();
    println!("保存済み作品の詳細表示");
    let default_workspace = Path::new(".").display().to_string();
    let workspace_input =
        prompt_with_default("workspace を入力してください", &default_workspace)?;
    let app = App::new(PathBuf::from(workspace_input))?;
    let records = app.list_records()?;
    if records.is_empty() {
        println!("保存済み作品はありません。");
        return Ok(());
    }
    print_records(&records, false);
    let id = prompt_single_id("詳細表示したい id を入力してください")?;
    let summary = app.inspect(&DownloadTarget::Id(id))?;
    println!("{}", summary.to_text());
    Ok(())
}

fn run_remove_menu() -> Result<()> {
    println!();
    println!("保存済み作品の削除");
    let default_workspace = Path::new(".").display().to_string();
    let workspace_input =
        prompt_with_default("workspace を入力してください", &default_workspace)?;
    let app = App::new(PathBuf::from(workspace_input))?;
    let records = app.list_records()?;
    if records.is_empty() {
        println!("保存済み作品はありません。");
        return Ok(());
    }
    print_records(&records, false);
    let id = prompt_single_id("削除したい id を入力してください")?;
    let remove_files = prompt_yes_no("作品フォルダも削除しますか？", false)?;
    let confirmed = prompt_yes_no("本当に削除しますか？", false)?;
    if !confirmed {
        println!("削除を中止しました。");
        return Ok(());
    }
    let summary = app.remove(&DownloadTarget::Id(id), remove_files)?;
    println!(
        "removed: id={} title={} files={}",
        summary.record.id, summary.record.title, summary.removed_files
    );
    Ok(())
}

fn run_doctor_menu() -> Result<()> {
    println!();
    println!("doctor");
    let default_workspace = Path::new(".").display().to_string();
    let workspace_input =
        prompt_with_default("workspace を入力してください", &default_workspace)?;
    let workspace = PathBuf::from(workspace_input);
    let aozora_dir = prompt_optional("AozoraEpub3 ディレクトリを入力してください（未入力なら自動探索）")?
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let app = App::new(workspace.clone())?;
    let records = app.list_records()?;
    if !records.is_empty() {
        print_records(&records, false);
    }
    let ids = prompt_optional("診断対象の id を空白区切りで入力してください（未入力で全件）")?
        .map(|raw| parse_id_list(&raw))
        .transpose()?
        .unwrap_or_default();
    let site = prompt_optional("サイト名で絞り込みますか？（未入力で全件）")?;
    let query = prompt_optional("タイトル/サイト/パスで検索しますか？（未入力で全件）")?;
    let json = prompt_yes_no("JSON で表示しますか？", false)?;
    let log_file = prompt_optional("ログファイルの出力先を入力してください（未入力なら画面表示のみ）")?
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let summary = run_doctor(
        &workspace,
        aozora_dir.as_deref(),
        DoctorOptions { ids, site, query },
    )?;
    let output = if json {
        serde_json::to_string_pretty(&summary)?
    } else {
        summary.to_text()
    };
    println!("{}", output);
    if let Some(path) = log_file {
        write_log_file(&path, &output)?;
    }
    Ok(())
}

async fn run_repair_menu() -> Result<()> {
    println!();
    println!("repair");
    let default_workspace = Path::new(".").display().to_string();
    let workspace_input =
        prompt_with_default("workspace を入力してください", &default_workspace)?;
    let workspace = PathBuf::from(workspace_input);
    let aozora_dir = prompt_optional("AozoraEpub3 ディレクトリを入力してください（未入力なら自動探索）")?
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let app = App::new(workspace.clone())?;
    let records = app.list_records()?;
    if !records.is_empty() {
        print_records(&records, false);
    }
    let ids = prompt_optional("修復対象の id を空白区切りで入力してください（未入力で全件）")?
        .map(|raw| parse_id_list(&raw))
        .transpose()?
        .unwrap_or_default();
    let site = prompt_optional("サイト名で絞り込みますか？（未入力で全件）")?;
    let query = prompt_optional("タイトル/サイト/パスで検索しますか？（未入力で全件）")?;
    let dry_run = prompt_yes_no("dry-run にしますか？", false)?;
    let prune = prompt_yes_no("孤立フォルダも掃除しますか？", false)?;
    let json = prompt_yes_no("JSON で表示しますか？", false)?;
    let log_file = prompt_optional("ログファイルの出力先を入力してください（未入力なら画面表示のみ）")?
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let summary = run_repair(
        &workspace,
        aozora_dir.as_deref(),
        RepairOptions {
            dry_run,
            prune,
            ids,
            site,
            query,
        },
    )
    .await?;
    let output = if json {
        serde_json::to_string_pretty(&summary)?
    } else {
        summary.to_text()
    };
    println!("{}", output);
    if let Some(path) = log_file {
        write_log_file(&path, &output)?;
    }
    Ok(())
}

fn prompt_common_options() -> Result<InteractiveSession> {
    let default_workspace = Path::new(".").display().to_string();
    let workspace_input =
        prompt_with_default("workspace を入力してください", &default_workspace)?;
    let workspace = PathBuf::from(workspace_input);
    let epub = prompt_yes_no("EPUB も生成しますか？", false)?;
    let aozora_dir = if epub {
        let value = prompt_optional("AozoraEpub3 ディレクトリを入力してください（未入力なら自動探索）")?;
        value.filter(|value| !value.trim().is_empty()).map(PathBuf::from)
    } else {
        None
    };
    Ok(InteractiveSession {
        workspace,
        epub,
        aozora_dir,
    })
}

fn prompt_menu(label: &str, max: usize) -> Result<usize> {
    loop {
        let value = prompt_non_empty(&format!("{label} [1-{max}]"))?;
        if let Ok(number) = value.parse::<usize>() {
            if (1..=max).contains(&number) {
                return Ok(number);
            }
        }
        println!("1 から {} の番号を入力してください。", max);
    }
}

fn prompt_yes_no(label: &str, default: bool) -> Result<bool> {
    let suffix = if default { "[Y/n]" } else { "[y/N]" };
    loop {
        let value = prompt_line(&format!("{label} {suffix}"))?;
        let trimmed = value.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return Ok(default);
        }
        match trimmed.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("y か n を入力してください。"),
        }
    }
}

fn prompt_with_default(label: &str, default: &str) -> Result<String> {
    let value = prompt_line(&format!("{label} [{default}]"))?;
    if value.trim().is_empty() {
        Ok(default.to_string())
    } else {
        Ok(value.trim().to_string())
    }
}

fn prompt_optional(label: &str) -> Result<Option<String>> {
    let value = prompt_line(label)?;
    if value.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(value.trim().to_string()))
    }
}

fn prompt_non_empty(label: &str) -> Result<String> {
    loop {
        let value = prompt_line(label)?;
        if !value.trim().is_empty() {
            return Ok(value.trim().to_string());
        }
        println!("入力してください。");
    }
}

fn parse_id_list(raw: &str) -> Result<Vec<u64>> {
    let mut ids = Vec::new();
    for token in raw.split_whitespace() {
        let id = token
            .parse::<u64>()
            .with_context(|| format!("invalid id: {token}"))?;
        ids.push(id);
    }
    if ids.is_empty() {
        bail!("id を1つ以上入力してください");
    }
    Ok(ids)
}

fn prompt_single_id(label: &str) -> Result<u64> {
    let raw = prompt_non_empty(label)?;
    raw.parse::<u64>()
        .with_context(|| format!("invalid id: {raw}"))
}

fn apply_record_filters(
    records: &mut Vec<narou_rust_core::NovelRecord>,
    site: Option<&str>,
    query: Option<&str>,
) {
    if let Some(site) = site {
        let site = site.trim().to_ascii_lowercase();
        if !site.is_empty() {
            records.retain(|record| record.sitename.to_ascii_lowercase().contains(&site));
        }
    }
    if let Some(query) = query {
        let query = query.trim().to_ascii_lowercase();
        if !query.is_empty() {
            records.retain(|record| {
                record.title.to_ascii_lowercase().contains(&query)
                    || record.author.to_ascii_lowercase().contains(&query)
                    || record.toc_url.to_ascii_lowercase().contains(&query)
            });
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum RecordSort {
    Id,
    Title,
    Site,
    Author,
    Episodes,
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

fn prompt_sort() -> Result<RecordSort> {
    println!("並び順を選んでください");
    println!("1. id");
    println!("2. title");
    println!("3. site");
    println!("4. author");
    println!("5. episodes");
    println!();
    match prompt_menu("並び順", 5)? {
        1 => Ok(RecordSort::Id),
        2 => Ok(RecordSort::Title),
        3 => Ok(RecordSort::Site),
        4 => Ok(RecordSort::Author),
        5 => Ok(RecordSort::Episodes),
        _ => unreachable!(),
    }
}

fn print_records(records: &[narou_rust_core::NovelRecord], verbose: bool) {
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

fn prompt_line(label: &str) -> Result<String> {
    print!("{label}: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input)
}

fn write_log_file(path: &Path, output: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, output)?;
    Ok(())
}
