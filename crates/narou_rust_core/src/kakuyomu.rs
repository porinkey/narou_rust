use crate::config::AppConfig;
use crate::model::{ParsedNovel, ResolvedNovel, Section, SectionElement, Subtitle};
use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;
use url::Url;

const BASE_URL: &str = "https://kakuyomu.jp";
const SITE_NAME: &str = "カクヨム";

pub struct KakuyomuClient {
    client: Client,
    config: AppConfig,
    throttle: Mutex<ThrottleState>,
}

impl KakuyomuClient {
    pub fn new(config: AppConfig) -> Result<Self> {
        let client = Client::builder()
            .user_agent("narou_rust/0.1")
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            config,
            throttle: Mutex::new(ThrottleState::default()),
        })
    }

    pub fn supports(target: &crate::model::DownloadTarget) -> bool {
        match target {
            crate::model::DownloadTarget::Url(url) => parse_work_id(url).is_ok(),
            _ => false,
        }
    }

    pub fn resolve_target(&self, target: &crate::model::DownloadTarget) -> Result<ResolvedNovel> {
        let work_id = match target {
            crate::model::DownloadTarget::Url(url) => parse_work_id(url)?,
            crate::model::DownloadTarget::Ncode(_) => {
                bail!("kakuyomu target must be specified by URL")
            }
            crate::model::DownloadTarget::Id(_) => bail!("id target must be resolved from database first"),
        };

        Ok(ResolvedNovel {
            toc_url: format!("{BASE_URL}/works/{work_id}"),
            info_url: format!("{BASE_URL}/works/{work_id}"),
            base_url: BASE_URL.to_string(),
        })
    }

    pub async fn fetch_novel(&self, resolved: &ResolvedNovel) -> Result<ParsedNovel> {
        let html = self.fetch_html(&resolved.toc_url).await?;
        let state = parse_next_data(&html)?;
        let work_id = parse_work_id(&resolved.toc_url)?;
        let state_map = state
            .get("props")
            .and_then(|value| value.get("pageProps"))
            .and_then(|value| value.get("__APOLLO_STATE__"))
            .and_then(Value::as_object)
            .context("__APOLLO_STATE__ not found")?;
        let work_key = format!("Work:{work_id}");
        let work = state_map
            .get(&work_key)
            .and_then(Value::as_object)
            .with_context(|| format!("work data not found: {work_key}"))?;

        let title = get_string(work, "title").unwrap_or_default();
        let author = extract_author(state_map, work)?;
        let story = get_string(work, "introduction").unwrap_or_default();
        let serial_status = get_string(work, "serialStatus").unwrap_or_default();
        let novel_type = 1;
        let end = serial_status == "COMPLETED";
        let episodes = extract_subtitles(state_map, work, &work_id)?;

        Ok(ParsedNovel {
            title,
            author,
            sitename: SITE_NAME.to_string(),
            story,
            toc_url: resolved.toc_url.clone(),
            episodes,
            novel_type,
            end,
            general_firstup: None,
            novelupdated_at: None,
            general_lastup: None,
            length: None,
        })
    }

    pub async fn fetch_section(&self, _toc_url: &str, subtitle: &Subtitle) -> Result<(Section, String)> {
        let url = if subtitle.href.starts_with("http://") || subtitle.href.starts_with("https://") {
            subtitle.href.clone()
        } else {
            format!("{BASE_URL}{}", subtitle.href)
        };
        let html = self.fetch_html(&url).await?;
        let page = Html::parse_document(&html);
        let body = extract_html(&page, ".widget-episodeBody.js-episode-body, .widget-episodeBody")
            .context("body not found")?;
        let section = Section {
            chapter: subtitle.chapter.clone(),
            subchapter: subtitle.subchapter.clone(),
            subtitle: subtitle.subtitle.clone(),
            element: SectionElement {
                data_type: "html".to_string(),
                introduction: String::new(),
                postscript: String::new(),
                body,
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

fn parse_work_id(input: &str) -> Result<String> {
    let url = Url::parse(input).with_context(|| format!("invalid url: {input}"))?;
    if url.host_str() != Some("kakuyomu.jp") {
        bail!("unsupported kakuyomu host");
    }
    let path = url.path().trim_end_matches('/');
    let work_re = Regex::new(r"^/works/(?P<id>\d+)(?:/episodes/\d+)?$").expect("valid regex");
    let captures = work_re
        .captures(path)
        .ok_or_else(|| anyhow!("kakuyomu work id not found in url: {input}"))?;
    Ok(captures["id"].to_string())
}

fn parse_next_data(html: &str) -> Result<Value> {
    let re = Regex::new(r#"(?s)<script id="__NEXT_DATA__" type="application/json">(?P<json>.+?)</script>"#)
        .expect("valid regex");
    let captures = re.captures(html).context("__NEXT_DATA__ not found")?;
    let json = captures.name("json").context("next data json not found")?.as_str();
    Ok(serde_json::from_str(json).context("failed to parse __NEXT_DATA__")?)
}

fn extract_author(
    state_map: &serde_json::Map<String, Value>,
    work: &serde_json::Map<String, Value>,
) -> Result<String> {
    let author_ref = work
        .get("author")
        .and_then(|value| value.get("__ref"))
        .and_then(Value::as_str)
        .context("author ref not found")?;
    let author = state_map
        .get(author_ref)
        .and_then(Value::as_object)
        .context("author data not found")?;
    let activity_name = get_string(author, "activityName").unwrap_or_default();
    if let Some(alternate) = get_string(work, "alternateAuthorName") {
        Ok(format!("{alternate}／{activity_name}"))
    } else {
        Ok(activity_name)
    }
}

fn extract_subtitles(
    state_map: &serde_json::Map<String, Value>,
    work: &serde_json::Map<String, Value>,
    work_id: &str,
) -> Result<Vec<Subtitle>> {
    let toc = work
        .get("tableOfContents")
        .and_then(Value::as_array)
        .context("tableOfContents not found")?;

    let mut subtitles = Vec::new();
    let mut emitted_chapter = String::new();
    for toc_item in toc {
        let toc_ref = toc_item
            .get("__ref")
            .and_then(Value::as_str)
            .context("tableOfContents ref not found")?;
        let toc_entry = state_map
            .get(toc_ref)
            .and_then(Value::as_object)
            .with_context(|| format!("toc entry not found: {toc_ref}"))?;

        let mut chapter = String::new();
        let mut subchapter = String::new();
        if let Some(chapter_ref) = toc_entry
            .get("chapter")
            .and_then(|value| value.get("__ref"))
            .and_then(Value::as_str)
        {
            if let Some(chapter_value) = state_map.get(chapter_ref).and_then(Value::as_object) {
                let title = get_string(chapter_value, "title").unwrap_or_default();
                match chapter_value.get("level").and_then(Value::as_u64) {
                    Some(2) => subchapter = title,
                    _ => chapter = title,
                }
            }
        }

        if let Some(episode_refs) = toc_entry.get("episodeUnions").and_then(Value::as_array) {
            for episode_ref in episode_refs {
                let episode_ref = episode_ref
                    .get("__ref")
                    .and_then(Value::as_str)
                    .context("episode ref not found")?;
                let episode = state_map
                    .get(episode_ref)
                    .and_then(Value::as_object)
                    .with_context(|| format!("episode not found: {episode_ref}"))?;
                let index = get_string(episode, "id").unwrap_or_default();
                let subtitle = get_string(episode, "title").unwrap_or_default();
                let published_at = get_string(episode, "publishedAt").unwrap_or_default();
                let episode_chapter = if !chapter.is_empty() && chapter != emitted_chapter {
                    emitted_chapter = chapter.clone();
                    chapter.clone()
                } else {
                    String::new()
                };
                subtitles.push(Subtitle {
                    index: index.clone(),
                    href: format!("/works/{work_id}/episodes/{index}"),
                    chapter: episode_chapter,
                    subchapter: subchapter.clone(),
                    subtitle: subtitle.clone(),
                    file_subtitle: crate::syosetu::sanitize_filename(&subtitle),
                    subdate: published_at.clone(),
                    subupdate: Some(published_at),
                });
            }
        }
    }
    Ok(subtitles)
}

fn get_string(map: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(|value| value.to_string())
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

#[cfg(test)]
mod tests {
    use super::parse_work_id;

    #[test]
    fn parses_work_url() {
        let id = parse_work_id("https://kakuyomu.jp/works/4852201425154905871").unwrap();
        assert_eq!(id, "4852201425154905871");
    }

    #[test]
    fn parses_episode_url() {
        let id = parse_work_id("https://kakuyomu.jp/works/4852201425154905871/episodes/4852201425154905928").unwrap();
        assert_eq!(id, "4852201425154905871");
    }
}
