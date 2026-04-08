use crate::model::{Section, Toc};
use crate::storage::Workspace;
use anyhow::{bail, Context, Result};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_AOZORA_DIR_NAME: &str = "AozoraEpub3-1.1.1b30Q";

#[derive(Debug, Clone)]
pub struct EpubSummary {
    pub txt_path: PathBuf,
    pub epub_path: PathBuf,
}

pub fn create_epub(
    workspace: &Workspace,
    sitename: &str,
    file_title: &str,
    explicit_aozora_dir: Option<&Path>,
) -> Result<EpubSummary> {
    let toc = workspace.load_toc(sitename, file_title)?;
    let novel_dir = workspace.novel_dir(sitename, file_title);
    let txt_path = novel_dir.join(format!("{file_title}.txt"));
    let text = build_aozora_text(workspace, sitename, file_title, &toc)?;
    workspace.write_text_at(&txt_path, &text)?;

    let aozora_dir = resolve_aozora_dir(workspace.root(), explicit_aozora_dir)?;
    let jar_path = aozora_dir.join("AozoraEpub3.jar");
    if !jar_path.exists() {
        bail!("AozoraEpub3.jar not found: {}", jar_path.display());
    }

    let before = find_epub_candidates(&novel_dir)?;
    let output = Command::new("java")
        .arg("-Dfile.encoding=UTF-8")
        .arg("-cp")
        .arg(jar_path.file_name().context("jar file name not found")?)
        .arg("AozoraEpub3")
        .arg("-enc")
        .arg("UTF-8")
        .arg("-dst")
        .arg(&novel_dir)
        .arg(&txt_path)
        .current_dir(&aozora_dir)
        .output()
        .context("failed to execute java/AozoraEpub3")?;

    if !output.status.success() {
        bail!(
            "AozoraEpub3 failed: status={} stdout={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let epub_path = detect_created_epub(&novel_dir, &before)
        .or_else(|| find_latest_epub(&novel_dir))
        .context("epub file not found after AozoraEpub3 execution")?;

    Ok(EpubSummary { txt_path, epub_path })
}

fn build_aozora_text(workspace: &Workspace, sitename: &str, file_title: &str, toc: &Toc) -> Result<String> {
    let novel_dir = workspace.novel_dir(sitename, file_title);
    let mut lines = Vec::new();
    lines.push(toc.title.trim_end().to_string());
    lines.push(toc.author.clone());
    lines.push(String::new());
    if let Some(cover_chuki) = create_cover_chuki(&novel_dir) {
        lines.push(cover_chuki);
    }
    lines.push(String::from("［＃区切り線］"));
    let story = postprocess_story(&html_to_text(&toc.story));
    if !story.trim().is_empty() {
        lines.push(String::from("あらすじ："));
        lines.push(story);
        lines.push(String::new());
    }
    lines.push(String::from("掲載ページ:"));
    lines.push(format!(r#"<a href="{0}">{0}</a>"#, toc.toc_url));
    lines.push(String::from("［＃区切り線］"));
    lines.push(String::new());

    let mut last_chapter = String::new();
    for subtitle in &toc.subtitles {
        let section = workspace.load_section(sitename, file_title, subtitle)?;
        append_section(&mut lines, toc.title.as_str(), &mut last_chapter, &section);
    }

    lines.push(String::new());
    lines.push(String::from("［＃ここから地付き］［＃小書き］（本を読み終わりました）［＃小書き終わり］［＃ここで地付き終わり］"));

    Ok(lines.join("\n"))
}

fn append_section(lines: &mut Vec<String>, title: &str, last_chapter: &mut String, section: &Section) {
    let chapter = section.chapter.trim_end();
    let mut page_break_inserted = false;
    if !chapter.is_empty() && *last_chapter != chapter {
        lines.push(String::from("［＃改ページ］"));
        lines.push(String::from("［＃ページの左右中央］"));
        lines.push(format!("［＃ここから柱］{}［＃ここで柱終わり］", title.trim_end()));
        lines.push(format!("［＃３字下げ］［＃大見出し］{}［＃大見出し終わり］", chapter));
        lines.push(String::from("［＃改ページ］"));
        *last_chapter = chapter.to_string();
        page_break_inserted = true;
    }

    if !page_break_inserted {
        lines.push(String::from("［＃改ページ］"));
    }
    if !section.subchapter.trim().is_empty() {
        lines.push(format!(
            "［＃１字下げ］［＃１段階大きな文字］{}［＃大きな文字終わり］",
            section.subchapter.trim()
        ));
    }
    lines.push(String::new());
    lines.push(format!(
        "［＃３字下げ］［＃中見出し］{}［＃中見出し終わり］",
        postprocess_subtitle(section.subtitle.trim_end())
    ));
    lines.push(String::new());
    lines.push(String::new());

    let intro = postprocess_body(&html_to_text(&section.element.introduction));
    let (intro, intro_illusts) = extract_illustration_markers(&intro);
    if !intro.trim().is_empty() {
        lines.push(String::from("［＃ここから前書き］"));
        lines.push(intro);
        lines.push(String::from("［＃ここで前書き終わり］"));
        if !intro_illusts.is_empty() {
            lines.extend(intro_illusts);
        }
        lines.push(String::new());
    }

    let body = postprocess_body(&html_to_text(&section.element.body));
    if !body.trim().is_empty() {
        lines.push(body);
        lines.push(String::new());
    }

    let postscript = postprocess_body(&html_to_text(&section.element.postscript));
    let (postscript, postscript_illusts) = extract_illustration_markers(&postscript);
    if !postscript.trim().is_empty() {
        lines.push(String::from("［＃ここから後書き］"));
        lines.push(postscript);
        lines.push(String::from("［＃ここで後書き終わり］"));
        if !postscript_illusts.is_empty() {
            lines.extend(postscript_illusts);
        }
        lines.push(String::new());
    }
}

fn html_to_text(input: &str) -> String {
    let mut text = input.replace("\r\n", "\n").replace('\r', "\n");
    let paragraph_close_re = Regex::new(r"(?is)\n?</p>").expect("valid regex");
    let ruby_re = Regex::new(r"(?is)<ruby>(.+?)</ruby>").expect("valid regex");
    let rt_split_re = Regex::new(r"(?is)<rt>").expect("valid regex");
    let rp_split_re = Regex::new(r"(?is)<rp>").expect("valid regex");
    let em_re =
        Regex::new(r#"(?is)<em\s+class=["']emphasisDots["']>(.+?)</em>"#).expect("valid regex");
    let img_re = Regex::new(r#"(?is)<img[^>]+src=["'](?P<src>.+?)["'][^>]*>"#).expect("valid regex");
    let br_re = Regex::new(r"(?is)<br\s*/?>").expect("valid regex");
    let block_close_re = Regex::new(r"(?is)</(div|li|h[1-6]|tr)>").expect("valid regex");
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("valid regex");

    text = text.replace('《', "≪").replace('》', "≫");
    text = paragraph_close_re.replace_all(&text, "\n").into_owned();
    text = ruby_re
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            let inner = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let split = rt_split_re.splitn(inner, 2).collect::<Vec<_>>();
            if split.len() < 2 {
                return strip_tags(split.first().copied().unwrap_or_default());
            }
            let ruby_base = strip_tags(rp_split_re.splitn(split[0], 2).next().unwrap_or_default());
            let ruby_text = strip_tags(rp_split_re.splitn(split[1], 2).next().unwrap_or_default());
            if ruby_text.trim().is_empty() {
                ruby_base
            } else {
                format!("｜{ruby_base}《{ruby_text}》")
            }
        })
        .into_owned();
    text = text.replace("<b>", "［＃太字］").replace("</b>", "［＃太字終わり］");
    text = text.replace("<B>", "［＃太字］").replace("</B>", "［＃太字終わり］");
    text = text.replace("<i>", "［＃斜体］").replace("</i>", "［＃斜体終わり］");
    text = text.replace("<I>", "［＃斜体］").replace("</I>", "［＃斜体終わり］");
    text = text.replace("<s>", "［＃取消線］").replace("</s>", "［＃取消線終わり］");
    text = text.replace("<S>", "［＃取消線］").replace("</S>", "［＃取消線終わり］");
    text = em_re
        .replace_all(&text, "［＃傍点］$1［＃傍点終わり］")
        .into_owned();
    text = img_re
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            let src = caps.name("src").map(|m| m.as_str()).unwrap_or_default();
            format!("［＃挿絵（{src}）入る］")
        })
        .into_owned();
    text = br_re.replace_all(&text, "\n").into_owned();
    text = block_close_re.replace_all(&text, "\n").into_owned();
    text = tag_re.replace_all(&text, "").into_owned();
    text = decode_entities(&text);
    normalize_blank_lines(&text)
}

fn strip_tags(input: &str) -> String {
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("valid regex");
    tag_re.replace_all(input, "").into_owned()
}

fn decode_entities(input: &str) -> String {
    input
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

fn normalize_blank_lines(input: &str) -> String {
    let mut normalized = String::new();
    let mut blank_run = 0_u8;
    for line in input.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.trim().is_empty() {
            blank_run = blank_run.saturating_add(1);
            if blank_run <= 1 {
                normalized.push('\n');
            }
            continue;
        }
        blank_run = 0;
        normalized.push_str(trimmed_end);
        normalized.push('\n');
    }
    normalized.trim().to_string()
}

fn postprocess_story(input: &str) -> String {
    let mut result = input.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
    result = to_zenkaku_ascii_general(&result);
    result = convert_year_month_tcy(&result);
    result = convert_numbers(&result, NumberMode::Story);
    result = rebuild_kome(&result);
    result = convert_kome_ascii(&result);
    result = convert_punctuation_tcy(&result);
    result = normalize_story_note_spacing(&result);
    result = insert_separate_space(&result);
    normalize_blank_lines(&result)
}

fn postprocess_subtitle(input: &str) -> String {
    let mut result = rebuild_kome(input.trim_end());
    result = to_zenkaku_ascii_general(&result);
    result = convert_numbers(&result, NumberMode::Subtitle);
    result = convert_punctuation_tcy(&result);
    result.trim_end().to_string()
}

fn postprocess_body(input: &str) -> String {
    let mut result = input.to_string();
    result = to_zenkaku_ascii_general(&result);
    result = convert_year_month_tcy(&result);
    result = convert_body_numbers(&result);
    result = rebuild_kome(&result);
    result = convert_punctuation_tcy(&result);
    result = add_half_indent_bracket(&result);
    result = insert_separate_space(&result);
    result = insert_space_after_tcy_marker(&result);
    result = auto_indent_lines(&result);
    normalize_dialogue_spacing(&normalize_blank_lines(&result))
}

fn rebuild_kome(input: &str) -> String {
    input.replace("※", "※［＃米印、1-2-8］")
}

#[derive(Clone, Copy)]
enum NumberMode {
    Story,
    Subtitle,
}

fn convert_numbers(input: &str, mode: NumberMode) -> String {
    let re = Regex::new(r"[0-9０-９]+").expect("valid regex");
    re.replace_all(input, |caps: &regex::Captures<'_>| {
        let num = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
        let matched = caps.get(0).expect("matched digits");
        if is_inside_tcy(input, matched.start(), matched.end()) {
            return num.to_string();
        }
        let normalized = ascii_digits(num);
        match normalized.len() {
            2 => tcy(&normalized),
            3 if matches!(mode, NumberMode::Subtitle) => tcy(&normalized),
            _ => to_zenkaku_digits(&normalized),
        }
    })
    .into_owned()
}

fn convert_body_numbers(input: &str) -> String {
    let re = Regex::new(r"[0-9０-９]+").expect("valid regex");
    re.replace_all(input, |caps: &regex::Captures<'_>| {
        let num = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
        let matched = caps.get(0).expect("matched digits");
        if is_inside_tcy(input, matched.start(), matched.end()) {
            return num.to_string();
        }
        if is_surrounded_by_ascii(input, matched.start(), matched.end()) {
            return num.to_string();
        }
        number_to_kanji(&ascii_digits(num))
    })
    .into_owned()
}

fn is_inside_tcy(input: &str, start: usize, end: usize) -> bool {
    let before = &input[..start];
    let after = &input[end..];
    let last_open = before.rfind("［＃縦中横］");
    let last_close = before.rfind("［＃縦中横終わり］");
    match (last_open, last_close) {
        (Some(open), Some(close)) if close > open => false,
        (Some(_), _) => after.contains("［＃縦中横終わり］"),
        _ => false,
    }
}

fn tcy(input: &str) -> String {
    format!("［＃縦中横］{input}［＃縦中横終わり］")
}

fn convert_year_month_tcy(input: &str) -> String {
    let re = Regex::new(r"(?P<year>[0-9０-９]{4})[\.．](?P<month>[0-9０-９]{2})").expect("valid regex");
    re.replace_all(input, |caps: &regex::Captures<'_>| {
        let year = to_zenkaku_digits(&ascii_digits(&caps["year"]));
        let month = ascii_digits(&caps["month"]);
        format!("{year}・{}{}", tcy(&month), "")
    })
    .into_owned()
}

fn ascii_digits(input: &str) -> String {
    input.chars().map(|ch| match ch {
        '０' => '0',
        '１' => '1',
        '２' => '2',
        '３' => '3',
        '４' => '4',
        '５' => '5',
        '６' => '6',
        '７' => '7',
        '８' => '8',
        '９' => '9',
        _ => ch,
    }).collect()
}

fn to_zenkaku_digits(input: &str) -> String {
    input.chars().map(|ch| match ch {
        '0' => '０',
        '1' => '１',
        '2' => '２',
        '3' => '３',
        '4' => '４',
        '5' => '５',
        '6' => '６',
        '7' => '７',
        '8' => '８',
        '9' => '９',
        _ => ch,
    }).collect()
}

fn number_to_kanji(input: &str) -> String {
    input.chars().map(|ch| match ch {
        '0' => '〇',
        '1' => '一',
        '2' => '二',
        '3' => '三',
        '4' => '四',
        '5' => '五',
        '6' => '六',
        '7' => '七',
        '8' => '八',
        '9' => '九',
        _ => ch,
    }).collect()
}

fn is_surrounded_by_ascii(input: &str, start: usize, end: usize) -> bool {
    let before = input[..start].chars().next_back();
    let after = input[end..].chars().next();
    before.map(is_ascii_word).unwrap_or(false) || after.map(is_ascii_word).unwrap_or(false)
}

fn is_ascii_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
}

fn convert_kome_ascii(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            if line.starts_with("※［＃米印、1-2-8］") {
                let prefix = "※［＃米印、1-2-8］";
                let rest = &line[prefix.len()..];
                format!("{prefix}{}", to_zenkaku_ascii(rest))
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_story_note_spacing(input: &str) -> String {
    let re = Regex::new(r"(?m)^(※［＃米印、1-2-8］.+?！)(２０\d\d・)").expect("valid regex");
    re.replace_all(input, "$1　$2").into_owned()
}

fn to_zenkaku_ascii_general(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' => char::from_u32(ch as u32 + 0xFEE0).unwrap_or(ch),
            '!' => '！',
            '"' => '”',
            '#' => '＃',
            '$' => '＄',
            '%' => '％',
            '&' => '＆',
            '\'' => '’',
            '(' => '（',
            ')' => '）',
            '*' => '＊',
            '+' => '＋',
            ',' => '，',
            '-' => '－',
            '.' => '．',
            '/' => '／',
            ':' => '：',
            ';' => '；',
            '<' => '＜',
            '=' => '＝',
            '>' => '＞',
            '?' => '？',
            '@' => '＠',
            _ => ch,
        })
        .collect()
}

fn convert_punctuation_tcy(input: &str) -> String {
    let exclamation_re = Regex::new(r"！{3,}").expect("valid regex");
    let mut result = exclamation_re
        .replace_all(input, |caps: &regex::Captures<'_>| {
            let len = caps.get(0).map(|m| m.as_str().chars().count()).unwrap_or(0);
            if len == 3 {
                tcy("!!!")
            } else {
                let normalized_len = if len % 2 == 0 { len } else { len + 1 };
                tcy("!!").repeat(normalized_len / 2)
            }
        })
        .into_owned();

    let mixed_re = Regex::new(r"[！？]{2,3}").expect("valid regex");
    result = mixed_re
        .replace_all(&result, |caps: &regex::Captures<'_>| {
            let text = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
            match text.chars().count() {
                2 => tcy(&text.replace('！', "!").replace('？', "?")),
                3 if text == "！！？" || text == "？！！" => {
                    tcy(&text.replace('！', "!").replace('？', "?"))
                }
                _ => text.to_string(),
            }
        })
        .into_owned();

    result
}

fn to_zenkaku_ascii(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            '!' => '！',
            '"' => '”',
            '#' => '＃',
            '$' => '＄',
            '%' => '％',
            '&' => '＆',
            '\'' => '’',
            '(' => '（',
            ')' => '）',
            '*' => '＊',
            '+' => '＋',
            ',' => '，',
            '-' => '－',
            '.' => '．',
            '/' => '／',
            ':' => '：',
            ';' => '；',
            '<' => '＜',
            '=' => '＝',
            '>' => '＞',
            '?' => '？',
            '@' => '＠',
            'A'..='Z' | 'a'..='z' => char::from_u32(ch as u32 + 0xFEE0).unwrap_or(ch),
            _ => ch,
        })
        .collect()
}

fn insert_separate_space(input: &str) -> String {
    let mut result = String::new();
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        result.push(ch);

        if is_sentence_punctuation(ch) {
            let mut run_end = index + 1;
            while run_end < chars.len() && is_sentence_punctuation(chars[run_end]) {
                result.push(chars[run_end]);
                run_end += 1;
            }

            if run_end < chars.len() {
                let next = chars[run_end];
                if should_insert_space_after_punctuation(next) {
                    result.push('　');
                }
            }
            index = run_end;
            continue;
        }

        index += 1;
    }

    result
}

fn add_half_indent_bracket(input: &str) -> String {
    let re = Regex::new(r#"(?m)^[ \t　]*([〔「『\(（【〈《≪〝])"#).expect("valid regex");
    re.replace_all(input, "［＃二分アキ］$1").into_owned()
}

fn auto_indent_lines(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            let trimmed = line.trim_start_matches([' ', '\t', '　']);
            if trimmed.is_empty()
                || trimmed.starts_with("［＃")
                || trimmed.starts_with("　")
                || trimmed.starts_with("［＃二分アキ］")
            {
                line.to_string()
            } else {
                format!("　{trimmed}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn insert_space_after_tcy_marker(input: &str) -> String {
    let marker = "［＃縦中横終わり］";
    let mut result = String::new();
    let mut rest = input;

    while let Some(index) = rest.find(marker) {
        let (before, after_marker) = rest.split_at(index);
        result.push_str(before);
        result.push_str(marker);
        let after = &after_marker[marker.len()..];
        if let Some(next) = after.chars().next() {
            if should_insert_space_after_tcy(next) {
                result.push('　');
            }
        }
        rest = after;
    }

    result.push_str(rest);
    result
}

fn is_sentence_punctuation(ch: char) -> bool {
    matches!(ch, '!' | '?' | '！' | '？')
}

fn should_insert_space_after_punctuation(ch: char) -> bool {
    !matches!(
        ch,
        '!' | '?' | '！' | '？'
            | '」' | '］' | '｝' | '}' | ']' | '』' | '】' | '〉' | '》' | '〕' | '＞' | '>' | '≫'
            | '）' | '"' | '”' | '’' | '〟'
            | '　' | '☆' | '★' | '♪' | '［' | '―' | '\n'
    )
}

fn should_insert_space_after_tcy(ch: char) -> bool {
    !matches!(
        ch,
        '　' | '\n' | '」' | '』' | '）' | '］' | '】' | '〉' | '》' | '〕' | '!' | '?' | '！' | '？'
    )
}

fn normalize_dialogue_spacing(input: &str) -> String {
    let mut lines = Vec::new();
    let source = input.lines().collect::<Vec<_>>();
    for (index, line) in source.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            let prev = index.checked_sub(1).and_then(|i| source.get(i)).map(|s| s.trim()).unwrap_or("");
            let next = source.get(index + 1).map(|s| s.trim()).unwrap_or("");
            if (is_dialogue_line(prev) || is_narrative_line(prev)) && (is_dialogue_line(next) || is_narrative_line(next)) {
                continue;
            }
        }
        lines.push((*line).to_string());
    }
    normalize_blank_lines(&lines.join("\n"))
}

fn is_dialogue_line(line: &str) -> bool {
    line.starts_with("［＃二分アキ］")
        || line.starts_with('「')
        || line.starts_with('『')
        || line.starts_with('（')
        || line.starts_with('〈')
}

fn is_narrative_line(line: &str) -> bool {
    !line.is_empty() && !line.starts_with("［＃")
}

fn create_cover_chuki(novel_dir: &Path) -> Option<String> {
    ["cover.jpg", "cover.png", "cover.jpeg"]
        .iter()
        .find(|name| novel_dir.join(name).exists())
        .map(|name| format!("［＃挿絵（{}）入る］", name))
}

fn extract_illustration_markers(input: &str) -> (String, Vec<String>) {
    let marker_re = Regex::new(r"^［＃挿絵（.+?）入る］$").expect("valid regex");
    let mut body_lines = Vec::new();
    let mut markers = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        if marker_re.is_match(trimmed) {
            markers.push(trimmed.to_string());
        } else {
            body_lines.push(line);
        }
    }
    (normalize_blank_lines(&body_lines.join("\n")), markers)
}

pub fn resolve_aozora_dir(workspace_root: &Path, explicit_aozora_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit_aozora_dir {
        return Ok(path.to_path_buf());
    }

    let direct = workspace_root.join(DEFAULT_AOZORA_DIR_NAME);
    if direct.exists() {
        return Ok(direct);
    }

    if let Some(parent) = workspace_root.parent() {
        let sibling = parent.join(DEFAULT_AOZORA_DIR_NAME);
        if sibling.exists() {
            return Ok(sibling);
        }
    }

    bail!(
        "AozoraEpub3 directory not found. Specify --aozora-dir or place {} next to the workspace",
        DEFAULT_AOZORA_DIR_NAME
    )
}

fn find_epub_candidates(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if !dir.exists() {
        return Ok(paths);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()).map(|ext| ext.eq_ignore_ascii_case("epub")) == Some(true) {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn detect_created_epub(dir: &Path, before: &[PathBuf]) -> Option<PathBuf> {
    let after = find_epub_candidates(dir).ok()?;
    after.into_iter().find(|path| !before.contains(path))
}

fn find_latest_epub(dir: &Path) -> Option<PathBuf> {
    let mut files = find_epub_candidates(dir).ok()?;
    files.sort_by_key(|path| std::fs::metadata(path).and_then(|meta| meta.modified()).ok());
    files.pop()
}

#[cfg(test)]
mod tests {
    use super::{
        add_half_indent_bracket, auto_indent_lines, convert_body_numbers, convert_numbers,
        convert_year_month_tcy, extract_illustration_markers, html_to_text, insert_separate_space,
        postprocess_body, postprocess_story, to_zenkaku_ascii, NumberMode,
    };

    #[test]
    fn converts_ruby_to_aozora_style() {
        let input = "<ruby>魔法<rt>まほう</rt></ruby>";
        assert_eq!(html_to_text(input), "｜魔法《まほう》");
    }

    #[test]
    fn converts_decorations_and_images() {
        let input = "<b>太字</b><br><i>斜体</i><br><s>取消</s><br><em class=\"emphasisDots\">傍点</em><br><img src=\"https://example.com/a.png\">";
        assert_eq!(
            html_to_text(input),
            "［＃太字］太字［＃太字終わり］\n［＃斜体］斜体［＃斜体終わり］\n［＃取消線］取消［＃取消線終わり］\n［＃傍点］傍点［＃傍点終わり］\n［＃挿絵（https://example.com/a.png）入る］"
        );
    }

    #[test]
    fn extracts_illustration_markers_from_author_comment() {
        let input = "本文\n［＃挿絵（cover.jpg）入る］\nあとがき";
        let (body, markers) = extract_illustration_markers(input);
        assert_eq!(body, "本文\nあとがき");
        assert_eq!(markers, vec!["［＃挿絵（cover.jpg）入る］"]);
    }

    #[test]
    fn adds_half_indent_to_dialogue_line() {
        assert_eq!(
            add_half_indent_bracket("「こんにちは」\n本文"),
            "［＃二分アキ］「こんにちは」\n本文"
        );
    }

    #[test]
    fn converts_year_month_to_ruby_style() {
        assert_eq!(
            convert_year_month_tcy("2024.11"),
            "２０２４・［＃縦中横］11［＃縦中横終わり］"
        );
    }

    #[test]
    fn converts_two_digit_numbers_to_tcy() {
        assert_eq!(
            convert_numbers("11", NumberMode::Story),
            "［＃縦中横］11［＃縦中横終わり］"
        );
    }

    #[test]
    fn postprocesses_story_notes() {
        let input = "※comicスピラ様にコミカライズしていただきました！2024.11";
        assert_eq!(
            postprocess_story(input),
            "※［＃米印、1-2-8］ｃｏｍｉｃスピラ様にコミカライズしていただきました！　２０２４・［＃縦中横］11［＃縦中横終わり］"
        );
    }

    #[test]
    fn converts_body_numbers_like_ruby() {
        assert_eq!(convert_body_numbers("17歳と2つ上"), "一七歳と二つ上");
        assert_eq!(convert_body_numbers("２つ上"), "二つ上");
    }

    #[test]
    fn converts_ascii_note_to_zenkaku() {
        assert_eq!(to_zenkaku_ascii("comic 2024"), "ｃｏｍｉｃ 2024");
    }

    #[test]
    fn adds_half_indent_in_body_postprocess() {
        assert_eq!(
            postprocess_body("「こんにちは」\n\n本文"),
            "［＃二分アキ］「こんにちは」\n　本文"
        );
    }

    #[test]
    fn auto_indents_narrative_lines() {
        assert_eq!(auto_indent_lines("本文\n［＃二分アキ］「台詞」"), "　本文\n［＃二分アキ］「台詞」");
    }

    #[test]
    fn inserts_space_after_punctuation() {
        assert_eq!(insert_separate_space("えっ！？あ"), "えっ！？　あ");
    }
}
