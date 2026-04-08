use crate::config::AppConfig;
use crate::model::{ParsedNovel, ResolvedNovel, Section, SectionElement, Subtitle};
use anyhow::{bail, Context, Result};
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE};
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;
use url::Url;

const BASE_URL: &str = "https://ncode.syosetu.com";
const DEFAULT_SITE_NAME: &str = "小説家になろう";
const ADULT_SITE_NAME: &str = "ノクターン・ムーンライト";

pub struct SyosetuClient {
    client: Client,
    config: AppConfig,
    throttle: Mutex<ThrottleState>,
}

impl SyosetuClient {
    pub fn new(config: AppConfig) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("over18=yes"));
        let client = Client::builder()
            .user_agent("narou_rust/0.1")
            .default_headers(headers)
            .cookie_store(true)
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            config,
            throttle: Mutex::new(ThrottleState::default()),
        })
    }

    pub fn resolve_target(&self, target: &crate::model::DownloadTarget) -> Result<ResolvedNovel> {
        let ncode_re = Regex::new(r"(?i)n\d+[a-z]+").expect("valid regex");
        let (ncode, base_url) = match target {
            crate::model::DownloadTarget::Ncode(value) => (value.clone(), BASE_URL.to_string()),
            crate::model::DownloadTarget::Url(url) => {
                let ncode = ncode_re
                    .find(url)
                    .map(|m| m.as_str().to_ascii_lowercase())
                    .ok_or_else(|| anyhow::anyhow!("ncode not found in url: {url}"))?;
                let base_url = parse_supported_base_url(url)?;
                (ncode, base_url)
            }
            crate::model::DownloadTarget::Id(_) => bail!("id target must be resolved from database first"),
        };
        Ok(ResolvedNovel {
            info_url: format!("{base_url}/novelview/infotop/ncode/{ncode}/"),
            toc_url: format!("{base_url}/{ncode}/"),
            base_url,
        })
    }

    pub async fn fetch_novel(&self, resolved: &ResolvedNovel) -> Result<ParsedNovel> {
        let mut episodes = Vec::new();
        let mut next_url = Some(resolved.toc_url.clone());
        let mut title = String::new();

        while let Some(url) = next_url.take() {
            let html = self.fetch_html(&url).await?;
            let page = Html::parse_document(&html);
            if title.is_empty() {
                title = page_title(&page);
            }
            episodes.extend(parse_toc_page(&page));
            next_url = next_toc_page(&page, &resolved.base_url);
        }

        let info_html = self.fetch_html(&resolved.info_url).await?;
        let info_page = Html::parse_document(&info_html);
        let author = extract_text(&info_page, "a.p-infotop-author__link, dd.p-infotop-data__value a").unwrap_or_default();
        let story = html_to_story_text(
            &extract_html(&info_page, "dd.p-infotop-data__value").unwrap_or_default(),
        );
        let novel_type_label = extract_text(&info_page, ".p-infotop-type__type, #noveltype").unwrap_or_default();
        let novel_type = if novel_type_label.contains("短編") { 2 } else { 1 };
        let end = novel_type_label.contains("完結");
        let info_map = extract_info_map(&info_page);

        if episodes.is_empty() && novel_type == 2 {
            episodes.push(create_short_story_subtitle(&title, &info_map));
        }

        let sitename = detect_site_name(&info_page, &info_map, &resolved.base_url);

        Ok(ParsedNovel {
            title,
            author,
            sitename,
            story,
            toc_url: resolved.toc_url.clone(),
            episodes,
            novel_type,
            end,
            general_firstup: info_map.get("掲載日").cloned(),
            novelupdated_at: info_map.get("最終更新日").cloned(),
            general_lastup: info_map
                .get("最新掲載日")
                .cloned()
                .or_else(|| info_map.get("最終掲載日").cloned())
                .or_else(|| info_map.get("最終更新日").cloned()),
            length: info_map
                .get("文字数")
                .and_then(|value| value.replace(',', "").parse::<usize>().ok()),
        })
    }

    pub async fn fetch_section(&self, toc_url: &str, subtitle: &Subtitle) -> Result<(Section, String)> {
        let base_url = base_url_from_url(toc_url)?;
        let url = if subtitle.href.is_empty() {
            toc_url.to_string()
        } else if subtitle.href.starts_with("http://") || subtitle.href.starts_with("https://") {
            subtitle.href.clone()
        } else if subtitle.href.starts_with('/') {
            format!("{base_url}{}", subtitle.href)
        } else {
            format!("{}{}", toc_url.trim_end_matches('/'), subtitle.href)
        };
        let html = self.fetch_html(&url).await?;
        let page = Html::parse_document(&html);
        let section = Section {
            chapter: subtitle.chapter.clone(),
            subchapter: subtitle.subchapter.clone(),
            subtitle: subtitle.subtitle.clone(),
            element: SectionElement {
                data_type: "html".to_string(),
                introduction: extract_html(&page, ".p-novel__text--preface").unwrap_or_default(),
                postscript: extract_html(&page, ".p-novel__text--afterword").unwrap_or_default(),
                body: extract_main_body_html(&page).context("body not found")?,
            },
        };
        Ok((section, html))
    }

    async fn fetch_html(&self, url: &str) -> Result<String> {
        let mut remaining_retries = self.config.retry_limit;

        loop {
            self.wait_for_request_slot().await;
            let response = self.client.get(url).send().await;
            match response {
                Ok(response) => {
                    let status = response.status();
                    if status.as_u16() == 404 {
                        bail!("request failed: {url} (404)");
                    }
                    if status.as_u16() == 503 {
                        bail!("request failed: {url} (503: access restricted or maintenance)");
                    }
                    let response = response
                        .error_for_status()
                        .with_context(|| format!("request failed: {url}"))?;
                    return Ok(response.text().await?);
                }
                Err(error) => {
                    if remaining_retries == 0 {
                        return Err(error).with_context(|| format!("request failed after retries: {url}"));
                    }
                    remaining_retries -= 1;
                    sleep(Duration::from_secs_f64(self.config.retry_wait_secs)).await;
                }
            }
        }
    }

    async fn wait_for_request_slot(&self) {
        let mut throttle = self.throttle.lock().await;
        let wait = throttle.next_wait(&self.config);
        if let Some(duration) = wait {
            sleep(duration).await;
        }
        throttle.mark_requested();
    }
}

#[derive(Debug)]
struct ThrottleState {
    request_count: u64,
    last_request_at: Option<Instant>,
}

impl Default for ThrottleState {
    fn default() -> Self {
        Self {
            request_count: 0,
            last_request_at: None,
        }
    }
}

impl ThrottleState {
    fn next_wait(&mut self, config: &AppConfig) -> Option<Duration> {
        let now = Instant::now();
        let regular_wait = Duration::from_secs_f64(config.download_interval_secs);
        let long_wait = Duration::from_secs_f64(config.long_wait_secs.max(config.download_interval_secs));

        if let Some(last_request_at) = self.last_request_at {
            if last_request_at.elapsed() > long_wait {
                self.request_count = 0;
            }
        }

        let target_wait = if self.request_count > 0
            && config.download_wait_steps > 0
            && self.request_count % config.download_wait_steps == 0
        {
            long_wait
        } else if self.request_count > 0 {
            regular_wait
        } else {
            Duration::ZERO
        };

        if target_wait.is_zero() {
            return None;
        }

        let elapsed = self
            .last_request_at
            .map(|last_request_at| now.saturating_duration_since(last_request_at))
            .unwrap_or(Duration::ZERO);

        if elapsed >= target_wait {
            None
        } else {
            Some(target_wait - elapsed)
        }
    }

    fn mark_requested(&mut self) {
        self.request_count += 1;
        self.last_request_at = Some(Instant::now());
    }
}

fn parse_toc_page(page: &Html) -> Vec<Subtitle> {
    let title_sel = Selector::parse("a.p-eplist__subtitle").expect("valid selector");
    let update_sel = Selector::parse(".p-eplist__update").expect("valid selector");
    let revise_sel = Selector::parse(".p-eplist__update span[title]").expect("valid selector");
    let mut current_chapter = String::new();
    let mut emitted_chapter = String::new();
    let mut subtitles = Vec::new();

    for node in page.tree.root().descendants() {
        if let Some(element) = scraper::ElementRef::wrap(node) {
            if element.value().name() == "div" && element.value().has_class("p-eplist__chapter-title", scraper::CaseSensitivity::CaseSensitive) {
                current_chapter = element.text().collect::<String>().trim().to_string();
            }
            if element.value().name() == "div" && element.value().has_class("p-eplist__sublist", scraper::CaseSensitivity::CaseSensitive) {
                let Some(title) = element.select(&title_sel).next() else {
                    continue;
                };
                let Some(href) = title.value().attr("href").map(|value| value.to_string()) else {
                    continue;
                };
                let Some(index) = href.trim_matches('/').split('/').next_back().map(|value| value.to_string()) else {
                    continue;
                };
                let subtitle_text = title.text().collect::<String>().trim().to_string();
                let update = element
                    .select(&update_sel)
                    .next()
                    .map(|node| node.text().collect::<String>())
                    .unwrap_or_default();
                let subdate = update
                    .split('（')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let subupdate = element
                    .select(&revise_sel)
                    .next()
                    .and_then(|node| node.value().attr("title"))
                    .map(|value| value.replace(" 改稿", ""));
                let chapter = if !current_chapter.is_empty() && current_chapter != emitted_chapter {
                    emitted_chapter = current_chapter.clone();
                    current_chapter.clone()
                } else {
                    String::new()
                };

                subtitles.push(Subtitle {
                    index,
                    href,
                    chapter,
                    subchapter: String::new(),
                    subtitle: subtitle_text.clone(),
                    file_subtitle: sanitize_filename(&subtitle_text),
                    subdate,
                    subupdate,
                });
            }
        }
    }

    subtitles
}

fn next_toc_page(page: &Html, base_url: &str) -> Option<String> {
    let next_sel = Selector::parse("a.c-pager__item--next").expect("valid selector");
    let href = page.select(&next_sel).next()?.value().attr("href")?;
    if href.starts_with("http://") || href.starts_with("https://") {
        Some(href.to_string())
    } else {
        Some(format!("{base_url}{href}"))
    }
}

fn page_title(page: &Html) -> String {
    extract_text(page, ".p-novel__title, .novel_title").unwrap_or_default()
}

fn extract_text(page: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    let node = page.select(&selector).next()?;
    let text = node.text().collect::<String>().trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn extract_html(page: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    let node = page.select(&selector).next()?;
    let html = node.inner_html().trim().to_string();
    if html.is_empty() {
        None
    } else {
        Some(html)
    }
}

fn html_to_story_text(input: &str) -> String {
    let text = input
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("valid regex");
    tag_re
        .replace_all(&text, "")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .trim()
        .to_string()
}

fn extract_main_body_html(page: &Html) -> Option<String> {
    let selector = Selector::parse(".js-novel-text.p-novel__text").ok()?;
    for node in page.select(&selector) {
        if is_preface_or_afterword(&node) {
            continue;
        }
        let html = node.inner_html().trim().to_string();
        if !html.is_empty() {
            return Some(html);
        }
    }
    None
}

fn is_preface_or_afterword(node: &ElementRef<'_>) -> bool {
    let mut classes = node.value().classes();
    classes.any(|class_name| {
        class_name == "p-novel__text--preface" || class_name == "p-novel__text--afterword"
    })
}

pub fn sanitize_filename(input: &str) -> String {
    let replaced = input
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            _ => ch,
        })
        .collect::<String>();
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim().to_string()
}

fn parse_supported_base_url(input: &str) -> Result<String> {
    let url = Url::parse(input).with_context(|| format!("invalid url: {input}"))?;
    let host = url.host_str().context("url host not found")?;
    if !is_supported_host(host) {
        bail!("unsupported syosetu host: {host}");
    }
    Ok(format!("{}://{}", url.scheme(), host))
}

fn base_url_from_url(input: &str) -> Result<String> {
    let url = Url::parse(input).with_context(|| format!("invalid url: {input}"))?;
    let host = url.host_str().context("url host not found")?;
    Ok(format!("{}://{}", url.scheme(), host))
}

fn is_supported_host(host: &str) -> bool {
    matches!(
        host,
        "ncode.syosetu.com" | "novel18.syosetu.com" | "noc.syosetu.com" | "mnlt.syosetu.com" | "mid.syosetu.com"
    )
}

fn detect_site_name(
    info_page: &Html,
    info_map: &std::collections::BTreeMap<String, String>,
    base_url: &str,
) -> String {
    if let Some(site) = info_map.get("掲載サイト").map(|value| normalize_site_name(value)) {
        if !site.is_empty() {
            return site;
        }
    }
    if let Some(site) = extract_meta_content(info_page, r#"meta[property="og:site_name"]"#) {
        return site;
    }
    if is_adult_base_url(base_url) {
        ADULT_SITE_NAME.to_string()
    } else {
        DEFAULT_SITE_NAME.to_string()
    }
}

fn normalize_site_name(input: &str) -> String {
    input
        .split(['(', '（'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn extract_meta_content(page: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    let node = page.select(&selector).next()?;
    let value = node.value().attr("content")?.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn is_adult_base_url(base_url: &str) -> bool {
    ["novel18.syosetu.com", "noc.syosetu.com", "mnlt.syosetu.com", "mid.syosetu.com"]
        .iter()
        .any(|host| base_url.contains(host))
}

#[cfg(test)]
mod tests {
    use super::{normalize_site_name, parse_supported_base_url};

    #[test]
    fn resolves_supported_r18_host() {
        let base_url = parse_supported_base_url("https://novel18.syosetu.com/n1610bw/").unwrap();
        assert_eq!(base_url, "https://novel18.syosetu.com");
    }

    #[test]
    fn strips_audience_suffix_from_site_name() {
        assert_eq!(normalize_site_name("ムーンライトノベルズ(女性向け)"), "ムーンライトノベルズ");
    }
}

fn create_short_story_subtitle(
    title: &str,
    info_map: &std::collections::BTreeMap<String, String>,
) -> Subtitle {
    let subdate = info_map.get("掲載日").cloned().unwrap_or_default();
    let subupdate = info_map
        .get("最終更新日")
        .cloned()
        .or_else(|| info_map.get("最新掲載日").cloned())
        .or_else(|| info_map.get("最終掲載日").cloned());

    Subtitle {
        index: "1".to_string(),
        href: String::new(),
        chapter: String::new(),
        subchapter: String::new(),
        subtitle: title.to_string(),
        file_subtitle: sanitize_filename(title),
        subdate,
        subupdate,
    }
}

fn extract_info_map(page: &Html) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    let row_selector = Selector::parse(".p-infotop-data__table dl").expect("valid selector");
    let dl_selector = Selector::parse("dl.p-infotop-data").expect("valid selector");
    let title_selector = Selector::parse(".p-infotop-data__title").expect("valid selector");
    let value_selector = Selector::parse(".p-infotop-data__value").expect("valid selector");

    for row in page.select(&row_selector) {
        let Some(key) = row
            .select(&title_selector)
            .next()
            .map(|node| node.text().collect::<String>().trim().to_string())
        else {
            continue;
        };
        let Some(value) = row
            .select(&value_selector)
            .next()
            .map(|node| node.text().collect::<String>().trim().to_string())
        else {
            continue;
        };
        map.insert(key, value);
    }

    if !map.is_empty() {
        return map;
    }

    if let Some(container) = page.select(&dl_selector).next() {
        let mut pending_key: Option<String> = None;
        for child in container.children() {
            let Some(element) = ElementRef::wrap(child) else {
                continue;
            };
            match element.value().name() {
                "dt" => {
                    let key = element.text().collect::<String>().trim().to_string();
                    if !key.is_empty() {
                        pending_key = Some(key);
                    }
                }
                "dd" => {
                    let Some(key) = pending_key.take() else {
                        continue;
                    };
                    let value = element.text().collect::<String>().trim().to_string();
                    map.insert(key, value);
                }
                _ => {}
            }
        }
    }

    map
}
