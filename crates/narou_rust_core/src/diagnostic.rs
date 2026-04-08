use anyhow::Error;

#[derive(Debug, Clone, Default)]
pub struct ErrorContext {
    pub command: Option<String>,
    pub target: Option<String>,
    pub workspace: Option<String>,
}

impl ErrorContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn command(mut self, value: impl Into<String>) -> Self {
        self.command = Some(value.into());
        self
    }

    pub fn target(mut self, value: impl Into<String>) -> Self {
        self.target = Some(value.into());
        self
    }

    pub fn workspace(mut self, value: impl Into<String>) -> Self {
        self.workspace = Some(value.into());
        self
    }
}

pub fn format_error_report(error: &Error, context: &ErrorContext) -> String {
    let primary = error.to_string();
    let causes = error
        .chain()
        .skip(1)
        .map(|cause| cause.to_string())
        .collect::<Vec<_>>();
    let code = classify_code(&primary, &causes);
    let stage = classify_stage(&primary, &causes);
    let hints = build_hints(&primary, &causes);

    let mut lines = vec![
        "error_report:".to_string(),
        format!("  code: {}", code),
        format!("  stage: {}", stage),
        format!("  summary: {}", yaml_quote(&primary)),
    ];

    if let Some(command) = &context.command {
        lines.push(format!("  command: {}", yaml_quote(command)));
    }
    if let Some(target) = &context.target {
        lines.push(format!("  target: {}", yaml_quote(target)));
    }
    if let Some(workspace) = &context.workspace {
        lines.push(format!("  workspace: {}", yaml_quote(workspace)));
    }

    if causes.is_empty() {
        lines.push("  causes: []".to_string());
    } else {
        lines.push("  causes:".to_string());
        for cause in causes {
            lines.push(format!("    - {}", yaml_quote(&cause)));
        }
    }

    lines.push("  hints:".to_string());
    for hint in hints {
        lines.push(format!("    - {}", yaml_quote(&hint)));
    }

    lines.join("\n")
}

fn classify_code(primary: &str, causes: &[String]) -> &'static str {
    let text = combined_text(primary, causes);
    if text.contains("target is empty") {
        "invalid_target.empty"
    } else if text.contains("unsupported target") {
        "invalid_target.unsupported"
    } else if text.contains("ncode not found in url") {
        "invalid_target.url_missing_ncode"
    } else if text.contains("record not found for id target") {
        "database.id_not_found"
    } else if text.contains("(404)") {
        "http.not_found"
    } else if text.contains("(503") {
        "http.access_restricted"
    } else if text.contains("request failed after retries") {
        "http.retry_exhausted"
    } else if text.contains("body not found") {
        "parse.section_body_missing"
    } else if text.contains("failed to read") {
        "filesystem.read_failed"
    } else if text.contains("failed to write") {
        "filesystem.write_failed"
    } else if text.contains("failed to parse") {
        "serialization.parse_failed"
    } else {
        "unknown"
    }
}

fn classify_stage(primary: &str, causes: &[String]) -> &'static str {
    let text = combined_text(primary, causes);
    if text.contains("target") || text.contains("ncode") {
        "input"
    } else if text.contains("request failed") || text.contains("(404)") || text.contains("(503") {
        "network"
    } else if text.contains("body not found") {
        "html_parse"
    } else if text.contains("failed to read") || text.contains("failed to write") {
        "filesystem"
    } else if text.contains("failed to parse") {
        "serialization"
    } else {
        "application"
    }
}

fn build_hints(primary: &str, causes: &[String]) -> Vec<String> {
    let text = combined_text(primary, causes);
    let mut hints = Vec::new();

    if text.contains("target is empty") || text.contains("unsupported target") {
        hints.push("CLI に渡した target が空でないか、URL / Nコード / ID のいずれかになっているか確認する".to_string());
    }
    if text.contains("ncode not found in url") {
        hints.push("入力URLに n1234ab の形式のNコードが含まれているか確認する".to_string());
        hints.push("短縮URLや作品情報URLではなく作品本文/目次URLを使う".to_string());
    }
    if text.contains("record not found for id target") {
        hints.push("指定した ID が .narou/database.yaml に存在するか確認する".to_string());
    }
    if text.contains("(404)") {
        hints.push("作品が削除・非公開・URL変更されていないか確認する".to_string());
    }
    if text.contains("(503") {
        hints.push("アクセス規制またはメンテナンスの可能性があるので時間を空けて再試行する".to_string());
        hints.push("download.interval と download.wait-steps を大きめに設定する".to_string());
    }
    if text.contains("request failed after retries") {
        hints.push("ネットワーク接続、TLS、対象サイトの応答状況を確認する".to_string());
        hints.push("download.retry-limit と download.retry-wait-seconds を調整する".to_string());
    }
    if text.contains("body not found") {
        hints.push("対象ページのHTML構造が変わっていないか確認する".to_string());
        hints.push("syosetu.rs の本文 selector を実ページHTMLに合わせて更新する".to_string());
    }
    if text.contains("failed to read") || text.contains("failed to write") {
        hints.push("workspace 配下のファイル/ディレクトリ権限とパスを確認する".to_string());
    }
    if text.contains("failed to parse") {
        hints.push("YAML の内容が壊れていないか、期待する形式か確認する".to_string());
    }
    if hints.is_empty() {
        hints.push("summary と causes を見て、失敗が input / network / html_parse / filesystem のどこで起きたか切り分ける".to_string());
        hints.push("再現入力、対象URL、保存先workspace を添えてAIに渡すと修正しやすい".to_string());
    }

    hints
}

fn combined_text(primary: &str, causes: &[String]) -> String {
    let mut text = primary.to_lowercase();
    for cause in causes {
        text.push('\n');
        text.push_str(&cause.to_lowercase());
    }
    text
}

fn yaml_quote(value: &str) -> String {
    format!("{:?}", value)
}
