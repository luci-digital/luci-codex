use crate::history_cell::CompositeHistoryCell;
use crate::history_cell::HistoryCell;
use crate::history_cell::PlainHistoryCell;
use crate::history_cell::SESSION_HEADER_MAX_INNER_WIDTH;
use crate::history_cell::card_inner_width;
use crate::history_cell::format_directory_display;
use crate::history_cell::with_border;
use crate::version::CODEX_CLI_VERSION;
use codex_common::create_config_summary_entries;
use codex_core::auth::get_auth_file;
use codex_core::auth::try_read_auth_json;
use codex_core::config::Config;
use codex_core::project_doc::discover_project_doc_paths;
use codex_core::protocol::RateLimitSnapshot;
use codex_core::protocol::SandboxPolicy;
use codex_core::protocol::TokenUsage;
use codex_protocol::mcp_protocol::ConversationId;
use ratatui::prelude::*;
use ratatui::style::Stylize;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

const STATUS_LIMIT_BAR_SEGMENTS: usize = 20;
const STATUS_LIMIT_BAR_FILLED: &str = "█";
const STATUS_LIMIT_BAR_EMPTY: &str = "░";

fn label_display(label: &str) -> String {
    format!(" {label}: ")
}

fn label_span(label: &str) -> Span<'static> {
    Span::from(label_display(label)).dim()
}

fn label_width(label: &str) -> usize {
    UnicodeWidthStr::width(label_display(label).as_str())
}

fn status_header_spans() -> Vec<Span<'static>> {
    vec![
        Span::from(">_ ").dim(),
        Span::from("OpenAI Codex").bold(),
        Span::from(" ").dim(),
        Span::from(format!("(v{CODEX_CLI_VERSION})")).dim(),
    ]
}

#[derive(Debug)]
struct StatusField {
    label: &'static str,
    value: Vec<Span<'static>>,
}

impl StatusField {
    fn text(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: vec![Span::from(value.into())],
        }
    }

    fn spans(label: &'static str, value: Vec<Span<'static>>) -> Self {
        Self { label, value }
    }
}

#[derive(Debug, Default)]
struct StatusRows {
    lines: Vec<Line<'static>>,
}

impl StatusRows {
    fn new() -> Self {
        Self { lines: Vec::new() }
    }

    fn push_blank(&mut self) {
        self.lines.push(Line::from(Vec::<Span<'static>>::new()));
    }

    fn push_line(&mut self, spans: Vec<Span<'static>>) {
        self.lines.push(Line::from(spans));
    }

    fn push_field(&mut self, field: StatusField) {
        let mut spans = Vec::with_capacity(field.value.len() + 1);
        spans.push(label_span(field.label));
        spans.extend(field.value);
        self.lines.push(Line::from(spans));
    }

    fn extend_fields<I>(&mut self, fields: I)
    where
        I: IntoIterator<Item = StatusField>,
    {
        for field in fields {
            self.push_field(field);
        }
    }

    fn extend_lines<I>(&mut self, lines: I)
    where
        I: IntoIterator<Item = Line<'static>>,
    {
        self.lines.extend(lines);
    }

    fn into_lines(self) -> Vec<Line<'static>> {
        self.lines
    }
}

pub(crate) fn new_status_output(
    config: &Config,
    usage: &TokenUsage,
    session_id: &Option<ConversationId>,
    rate_limits: Option<&RateLimitSnapshot>,
) -> CompositeHistoryCell {
    let command = PlainHistoryCell::new(vec!["/status".magenta().into()]);
    let card = StatusHistoryCell::new(config, usage, session_id, rate_limits);

    CompositeHistoryCell::new(vec![Box::new(command), Box::new(card)])
}

#[derive(Debug, Clone)]
struct StatusTokenUsageData {
    total: u64,
    input: u64,
    cached_input: u64,
    output: u64,
}

#[derive(Debug, Clone)]
enum StatusAccountDisplay {
    ChatGpt {
        email: Option<String>,
        plan: Option<String>,
    },
    ApiKey,
}

#[derive(Debug, Clone)]
struct StatusRateLimitRow {
    label: String,
    percent_used: f64,
}

#[derive(Debug, Clone)]
enum StatusRateLimitData {
    Available(Vec<StatusRateLimitRow>),
    Missing,
}

#[derive(Debug)]
struct StatusHistoryCell {
    model_display: String,
    directory: PathBuf,
    approval: String,
    sandbox: String,
    agents_summary: String,
    account: Option<StatusAccountDisplay>,
    session_id: Option<String>,
    token_usage: StatusTokenUsageData,
    rate_limits: StatusRateLimitData,
}

impl StatusHistoryCell {
    fn new(
        config: &Config,
        usage: &TokenUsage,
        session_id: &Option<ConversationId>,
        rate_limits: Option<&RateLimitSnapshot>,
    ) -> Self {
        let config_entries = create_config_summary_entries(config);
        let model_display = compose_model_display(config, &config_entries);
        let approval = config_entries
            .iter()
            .find(|(k, _)| *k == "approval")
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| "<unknown>".to_string());
        let sandbox = match &config.sandbox_policy {
            SandboxPolicy::DangerFullAccess => "danger-full-access".to_string(),
            SandboxPolicy::ReadOnly => "read-only".to_string(),
            SandboxPolicy::WorkspaceWrite { .. } => "workspace-write".to_string(),
        };
        let agents_summary = compose_agents_summary(config);
        let account = compose_account_display(config);
        let session_id = session_id.as_ref().map(std::string::ToString::to_string);
        let token_usage = StatusTokenUsageData {
            total: usage.blended_total(),
            input: usage.non_cached_input(),
            cached_input: usage.cached_input_tokens,
            output: usage.output_tokens,
        };
        let rate_limits = compose_rate_limit_data(rate_limits);

        Self {
            model_display,
            directory: config.cwd.clone(),
            approval,
            sandbox,
            agents_summary,
            account,
            session_id,
            token_usage,
            rate_limits,
        }
    }

    fn primary_fields(&self, inner_width: usize) -> Vec<StatusField> {
        let mut fields = Vec::new();
        fields.push(StatusField::text("Model", self.model_display.clone()));

        let directory_width = inner_width.saturating_sub(label_width("Directory"));
        let directory = format_directory_display(&self.directory, Some(directory_width));
        fields.push(StatusField::text("Directory", directory));

        fields.push(StatusField::text("Approval", self.approval.clone()));
        fields.push(StatusField::text("Sandbox", self.sandbox.clone()));
        fields.push(StatusField::text("Agents.md", self.agents_summary.clone()));

        fields
    }

    fn account_field(&self) -> Option<StatusField> {
        let account = self.account.as_ref()?;
        let value = match account {
            StatusAccountDisplay::ChatGpt { email, plan } => match (email, plan) {
                (Some(email), Some(plan)) => format!("{email} ({plan})"),
                (Some(email), None) => email.clone(),
                (None, Some(plan)) => plan.clone(),
                (None, None) => "ChatGPT".to_string(),
            },
            StatusAccountDisplay::ApiKey => {
                "API key configured (run codex login to use ChatGPT)".to_string()
            }
        };

        Some(StatusField::text("Account", value))
    }

    fn session_field(&self) -> Option<StatusField> {
        self.session_id
            .as_ref()
            .map(|session| StatusField::text("Session", session.clone()))
    }

    fn token_usage_field(&self) -> StatusField {
        StatusField::spans("Token Usage", self.token_usage_spans())
    }

    fn token_usage_spans(&self) -> Vec<Span<'static>> {
        let total_fmt = format_tokens_compact(self.token_usage.total);
        let input_fmt = format_tokens_compact(self.token_usage.input);
        let output_fmt = format_tokens_compact(self.token_usage.output);

        let mut spans: Vec<Span<'static>> = vec![
            Span::from(total_fmt),
            Span::from(" (").dim(),
            Span::from(input_fmt),
            Span::from(" input").dim(),
            Span::from(" + ").dim(),
            Span::from(output_fmt),
            Span::from(" output").dim(),
            Span::from(")").dim(),
        ];

        if self.token_usage.cached_input > 0 {
            let cached_fmt = format_tokens_compact(self.token_usage.cached_input);
            spans.push(Span::from(" + ").dim());
            spans.push(Span::from(format!("{cached_fmt} cached")).dim());
        }

        spans
    }

    fn rate_limit_lines(&self) -> Vec<Line<'static>> {
        match &self.rate_limits {
            StatusRateLimitData::Available(rows_data) => {
                let label_width = rows_data
                    .iter()
                    .map(|row| UnicodeWidthStr::width(row.label.as_str()))
                    .max()
                    .unwrap_or(0);

                rows_data
                    .iter()
                    .map(|row| {
                        let padded = format!("{label:<label_width$}", label = row.label);
                        Line::from(vec![
                            Span::from(format!(" {padded}: ")).dim(),
                            Span::from(render_status_limit_progress_bar(row.percent_used)),
                            Span::from(" "),
                            Span::from(format_status_limit_summary(row.percent_used)),
                        ])
                    })
                    .collect()
            }
            StatusRateLimitData::Missing => {
                vec![Line::from(vec![
                    label_span("Limits"),
                    Span::from("data not available yet").dim(),
                ])]
            }
        }
    }
}

impl HistoryCell for StatusHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let Some(inner_width) = card_inner_width(width, SESSION_HEADER_MAX_INNER_WIDTH) else {
            return Vec::new();
        };

        let mut rows = StatusRows::new();
        rows.push_line(status_header_spans());
        rows.push_blank();
        rows.extend_fields(self.primary_fields(inner_width));

        if let Some(account) = self.account_field() {
            rows.push_field(account);
        }

        if let Some(session) = self.session_field() {
            rows.push_field(session);
        }

        rows.push_blank();
        rows.push_field(self.token_usage_field());
        rows.extend_lines(self.rate_limit_lines());

        with_border(rows.into_lines())
    }
}

fn compose_model_display(config: &Config, entries: &[(&str, String)]) -> String {
    let mut details: Vec<String> = Vec::new();
    if let Some((_, effort)) = entries.iter().find(|(k, _)| *k == "reasoning effort") {
        details.push(format!("reasoning {}", title_case(effort)));
    }
    if entries
        .iter()
        .find(|(k, _)| *k == "reasoning summaries")
        .map(|(_, v)| v.eq_ignore_ascii_case("on"))
        .unwrap_or(false)
    {
        details.push("summaries on".to_string());
    }

    if details.is_empty() {
        config.model.clone()
    } else {
        format!("{} ({})", config.model, details.join(", "))
    }
}

fn compose_agents_summary(config: &Config) -> String {
    match discover_project_doc_paths(config) {
        Ok(paths) => {
            let mut rels: Vec<String> = Vec::new();
            for p in paths {
                let display = if let Some(parent) = p.parent() {
                    if parent == config.cwd {
                        "AGENTS.md".to_string()
                    } else {
                        let mut cur = config.cwd.as_path();
                        let mut ups = 0usize;
                        let mut reached = false;
                        while let Some(c) = cur.parent() {
                            if cur == parent {
                                reached = true;
                                break;
                            }
                            cur = c;
                            ups += 1;
                        }
                        if reached {
                            let up = format!("..{}", std::path::MAIN_SEPARATOR);
                            format!("{}AGENTS.md", up.repeat(ups))
                        } else if let Ok(stripped) = p.strip_prefix(&config.cwd) {
                            stripped.display().to_string()
                        } else {
                            p.display().to_string()
                        }
                    }
                } else {
                    p.display().to_string()
                };
                rels.push(display);
            }
            if rels.is_empty() {
                "<none>".to_string()
            } else {
                rels.join(", ")
            }
        }
        Err(_) => "<none>".to_string(),
    }
}

fn compose_account_display(config: &Config) -> Option<StatusAccountDisplay> {
    let auth_file = get_auth_file(&config.codex_home);
    let auth = try_read_auth_json(&auth_file).ok()?;

    if let Some(tokens) = auth.tokens.as_ref() {
        let info = &tokens.id_token;
        let email = info.email.clone();
        let plan = info.get_chatgpt_plan_type().map(|p| title_case(p.as_str()));
        return Some(StatusAccountDisplay::ChatGpt { email, plan });
    }

    if let Some(key) = auth.openai_api_key
        && !key.is_empty()
    {
        return Some(StatusAccountDisplay::ApiKey);
    }

    None
}

fn compose_rate_limit_data(snapshot: Option<&RateLimitSnapshot>) -> StatusRateLimitData {
    match snapshot {
        Some(snapshot) => StatusRateLimitData::Available(vec![
            StatusRateLimitRow {
                label: "5h Limit".to_string(),
                percent_used: snapshot.primary_used_percent,
            },
            StatusRateLimitRow {
                label: "Weekly Limit".to_string(),
                percent_used: snapshot.secondary_used_percent,
            },
        ]),
        None => StatusRateLimitData::Missing,
    }
}

fn format_tokens_compact(value: u64) -> String {
    if value == 0 {
        return "0".to_string();
    }
    if value < 1_000 {
        return value.to_string();
    }

    let (scaled, suffix) = if value >= 1_000_000_000_000 {
        (value as f64 / 1_000_000_000_000.0, "T")
    } else if value >= 1_000_000_000 {
        (value as f64 / 1_000_000_000.0, "B")
    } else if value >= 1_000_000 {
        (value as f64 / 1_000_000.0, "M")
    } else {
        (value as f64 / 1_000.0, "K")
    };

    let decimals = if scaled < 10.0 {
        2
    } else if scaled < 100.0 {
        1
    } else {
        0
    };

    let mut formatted = format!("{scaled:.decimals$}");
    if formatted.contains('.') {
        while formatted.ends_with('0') {
            formatted.pop();
        }
        if formatted.ends_with('.') {
            formatted.pop();
        }
    }

    format!("{formatted}{suffix}")
}

fn render_status_limit_progress_bar(percent_used: f64) -> String {
    let ratio = (percent_used / 100.0).clamp(0.0, 1.0);
    let filled = (ratio * STATUS_LIMIT_BAR_SEGMENTS as f64).round() as usize;
    let filled = filled.min(STATUS_LIMIT_BAR_SEGMENTS);
    let empty = STATUS_LIMIT_BAR_SEGMENTS.saturating_sub(filled);
    format!(
        "[{}{}]",
        STATUS_LIMIT_BAR_FILLED.repeat(filled),
        STATUS_LIMIT_BAR_EMPTY.repeat(empty)
    )
}

fn format_status_limit_summary(percent_used: f64) -> String {
    format!("{percent_used:.0}% used")
}

fn title_case(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return String::new(),
    };
    let rest: String = chars.as_str().to_ascii_lowercase();
    first.to_uppercase().collect::<String>() + &rest
}
