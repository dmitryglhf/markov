use anstream::{adapter::strip_str, println};
use bat::WrappingMode;
use console::{measure_text_width, style, Color, StyledObject, Term};
use goose::config::{Config, GooseMode};
use goose::conversation::message::{
    ActionRequiredData, Message, MessageContent, SystemNotificationContent, SystemNotificationType,
    ToolNameParts, ToolRequest, ToolResponse,
};
use goose::providers::canonical::maybe_get_canonical_model;
#[cfg(target_os = "windows")]
use goose::subprocess::SubprocessExt;
use goose::utils::safe_truncate;
use goose_providers::conversation::token_usage::Usage;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rmcp::model::{Annotations, CallToolRequestParams, JsonObject, PromptArgument, Role};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use std::io::{Error, IsTerminal, Write};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use strum::{EnumMessage, VariantNames};

use super::streaming_buffer::MarkdownBuffer;

pub const DEFAULT_MIN_PRIORITY: f32 = 0.0;
pub const DEFAULT_CLI_LIGHT_THEME: &str = "GitHub";
pub const DEFAULT_CLI_DARK_THEME: &str = "zenburn";

fn accent<T: Display>(value: T) -> StyledObject<T> {
    style(value).cyan()
}

fn warning<T: Display>(value: T) -> StyledObject<T> {
    style(value).yellow()
}

fn danger<T: Display>(value: T) -> StyledObject<T> {
    style(value).red()
}

// Re-export theme for use in main
#[derive(Clone, Copy)]
pub enum Theme {
    Light,
    Dark,
    Ansi,
}

impl Theme {
    fn as_str(&self) -> String {
        match self {
            Theme::Light => Config::global()
                .get_param::<String>("GOOSE_CLI_LIGHT_THEME")
                .unwrap_or(DEFAULT_CLI_LIGHT_THEME.to_string()),
            Theme::Dark => Config::global()
                .get_param::<String>("GOOSE_CLI_DARK_THEME")
                .unwrap_or(DEFAULT_CLI_DARK_THEME.to_string()),
            Theme::Ansi => "base16".to_string(),
        }
    }

    fn from_config_str(val: &str) -> Self {
        if val.eq_ignore_ascii_case("light") {
            Theme::Light
        } else if val.eq_ignore_ascii_case("ansi") {
            Theme::Ansi
        } else {
            Theme::Dark
        }
    }

    fn as_config_string(&self) -> String {
        match self {
            Theme::Light => "light".to_string(),
            Theme::Dark => "dark".to_string(),
            Theme::Ansi => "ansi".to_string(),
        }
    }
}

thread_local! {
    static CURRENT_THEME: RefCell<Theme> = RefCell::new(
        std::env::var("GOOSE_CLI_THEME").ok()
            .map(|val| Theme::from_config_str(&val))
            .unwrap_or_else(||
                Config::global().get_param::<String>("GOOSE_CLI_THEME").ok()
                    .map(|val| Theme::from_config_str(&val))
                    .unwrap_or(Theme::Ansi)
            )
    );
    static SHOW_FULL_TOOL_OUTPUT: RefCell<bool> = RefCell::new(
        Config::global().get_param::<bool>("GOOSE_SHOW_FULL_OUTPUT").unwrap_or(false)
    );
}

pub fn set_theme(theme: Theme) {
    let config = Config::global();
    config
        .set_param("GOOSE_CLI_THEME", theme.as_config_string())
        .expect("Failed to set theme");
    CURRENT_THEME.with(|t| *t.borrow_mut() = theme);

    let config = Config::global();
    let theme_str = match theme {
        Theme::Light => "light",
        Theme::Dark => "dark",
        Theme::Ansi => "ansi",
    };

    if let Err(e) = config.set_param("GOOSE_CLI_THEME", theme_str) {
        eprintln!("Failed to save theme setting to config: {}", e);
    }
}

pub fn get_theme() -> Theme {
    CURRENT_THEME.with(|t| *t.borrow())
}

pub fn toggle_full_tool_output() -> bool {
    SHOW_FULL_TOOL_OUTPUT.with(|s| {
        let mut val = s.borrow_mut();
        *val = !*val;
        *val
    })
}

pub fn get_show_full_tool_output() -> bool {
    SHOW_FULL_TOOL_OUTPUT.with(|s| *s.borrow())
}

// Simple wrapper around spinner to manage its state
#[derive(Default)]
pub struct ThinkingIndicator {
    spinner: Option<cliclack::ProgressBar>,
}

impl ThinkingIndicator {
    pub fn show(&mut self) {
        let spinner = cliclack::spinner();
        let hint = style("(Ctrl+C to interrupt)").dim();
        if Config::global()
            .get_param("RANDOM_THINKING_MESSAGES")
            .unwrap_or(true)
        {
            spinner.start(format!(
                "{}...  {}",
                super::thinking::get_random_thinking_message(),
                hint,
            ));
        } else {
            spinner.start(format!("Thinking...  {}", hint));
        }
        self.spinner = Some(spinner);
    }

    pub fn hide(&mut self) {
        if let Some(spinner) = self.spinner.take() {
            spinner.stop("");
        }
    }

    pub fn is_shown(&self) -> bool {
        self.spinner.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct PromptInfo {
    pub name: String,
    pub description: Option<String>,
    pub arguments: Option<Vec<PromptArgument>>,
    pub extension: Option<String>,
}

// Global thinking indicator
thread_local! {
    static THINKING: RefCell<ThinkingIndicator> = RefCell::new(ThinkingIndicator::default());
    static NEXT_MARKER: RefCell<Option<String>> = const { RefCell::new(None) };
    static SUBAGENT_RUN: RefCell<SubagentRun> = RefCell::new(SubagentRun::default());
    static TOOL_RUN: Cell<bool> = const { Cell::new(false) };
    static OUTPUT_HINT_SHOWN: Cell<bool> = const { Cell::new(false) };
    static FULL_RESULT_IDS: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
}

/// How many lines of output are short enough to be worth printing whole. Below
/// this a summary costs as much room as the thing it summarizes.
const COMPACT_OUTPUT_LINES: usize = 3;

/// How many calls back the results that must not be summarized are remembered.
/// Only the answer of a delegate is on that list, and one turn holds few of
/// those.
const FULL_RESULT_MEMORY: usize = 32;

/// How wide the label of a subagent is allowed to be. The core sends what the
/// task was asked to do, which is a sentence; on the line of a tool call there
/// is only room for the beginning of it.
const SUBAGENT_LABEL_BUDGET: usize = 24;

/// Tracks the delegates of one session that have spoken already, so that each
/// is named the same way every time it does.
#[derive(Default)]
struct SubagentRun {
    seen: Vec<String>,
}

impl SubagentRun {
    fn name(&mut self, subagent_id: &str, label: Option<&str>) -> String {
        let known = self.seen.iter().position(|id| id == subagent_id);
        let index = known.unwrap_or_else(|| {
            self.seen.push(subagent_id.to_string());
            self.seen.len() - 1
        });

        // a task that says what it is names itself; otherwise the delegates of
        // this session are numbered in the order they first speak, which is
        // still enough to tell two parallel streams apart
        let name = match label.map(str::trim).filter(|label| !label.is_empty()) {
            Some(label) => safe_truncate(label, SUBAGENT_LABEL_BUDGET),
            None => (index + 1).to_string(),
        };

        // the session id is spelled out once, so that the session a delegate
        // ran in can still be opened after the fact
        match known {
            Some(_) => format!("[{name}]"),
            None => format!("[{name}] {subagent_id}"),
        }
    }
}

/// Every block of output is separated from the one above by a single empty
/// line. A run of tool calls is one block however many calls it holds, whoever
/// makes them, so anything else that is printed ends the run.
fn open_block() {
    end_tool_run();
    println!();
}

fn end_tool_run() {
    TOOL_RUN.with(|open| open.set(false));
}

/// Opens the block a tool call belongs to, and says whether it had to be
/// opened. A call that joins a run already going adds no empty line of its own.
fn open_tool_run() -> bool {
    let opens = !TOOL_RUN.with(|open| open.replace(true));
    if opens {
        println!();
    }
    opens
}

fn subagent_label(subagent_id: &str, label: Option<&str>) -> String {
    SUBAGENT_RUN.with(|run| run.borrow_mut().name(subagent_id, label))
}

/// Columns a reply is moved in by, so that it lines up with the rest of the
/// output and leaves room for the marker of whoever is speaking.
const REPLY_INDENT: &str = "  ";

/// Marks the next block of markdown as the answer of the model.
pub fn begin_answer() {
    set_marker(style("●").dim().to_string());
}

/// Marks the next block of markdown as a message of the human, matching the
/// prompt they typed it at.
pub fn begin_user_message() {
    set_marker(style(">").green().to_string());
}

fn set_marker(marker: String) {
    NEXT_MARKER.with(|m| *m.borrow_mut() = Some(marker));
}

fn take_marker() -> Option<String> {
    NEXT_MARKER.with(|m| m.borrow_mut().take())
}

pub fn show_thinking() {
    if std::io::stdout().is_terminal() {
        THINKING.with(|t| t.borrow_mut().show());
    }
}

pub fn hide_thinking() {
    if std::io::stdout().is_terminal() {
        THINKING.with(|t| t.borrow_mut().hide());
    }
}

pub fn run_status_hook(status: &str) {
    if let Ok(hook) = Config::global().get_param::<String>("GOOSE_STATUS_HOOK") {
        let status = status.to_string();
        std::thread::spawn(move || {
            #[cfg(target_os = "windows")]
            let result = std::process::Command::new("cmd")
                .arg("/C")
                .arg(format!("{} {}", hook, status))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .set_no_window()
                .status();

            #[cfg(not(target_os = "windows"))]
            let result = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{} {}", hook, status))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            let _ = result;
        });
    }
}

pub fn is_showing_thinking() -> bool {
    THINKING.with(|t| t.borrow().is_shown())
}

pub fn set_thinking_message(s: &String) {
    if std::io::stdout().is_terminal() {
        THINKING.with(|t| {
            if let Some(spinner) = t.borrow_mut().spinner.as_mut() {
                spinner.set_message(s);
            }
        });
    }
}

pub fn render_message(message: &Message, debug: bool) {
    if !message.is_user_visible() {
        return;
    }
    let message = message.user_visible_content();
    let theme = get_theme();

    for content in &message.content {
        match content {
            MessageContent::ActionRequired(action) => match &action.data {
                ActionRequiredData::ToolConfirmation { tool_name, .. } => {
                    println!("action_required(tool_confirmation): {}", tool_name)
                }
                ActionRequiredData::Elicitation { message, .. } => {
                    println!("action_required(elicitation): {}", message)
                }
                ActionRequiredData::ElicitationResponse { id, .. } => {
                    println!("action_required(elicitation_response): {}", id)
                }
                ActionRequiredData::ToolConfirmationResponse { id, .. } => {
                    println!("action_required(tool_confirmation_response): {}", id)
                }
            },
            MessageContent::Text(text) => print_markdown(&text.text, theme),
            MessageContent::ToolRequest(req) => render_tool_request(req, theme, debug),
            MessageContent::ToolResponse(resp) => {
                render_tool_response(resp, debug);
                // whatever the model says next is it speaking again, not a
                // continuation of the tool it just called
                begin_answer();
            }
            MessageContent::Image(image) => {
                println!("Image: [data: {}, type: {}]", image.data, image.mime_type);
            }
            MessageContent::Thinking(t) => render_thinking(&t.thinking, theme),
            MessageContent::RedactedThinking(_) => {
                println!("\n{}", style("Thinking:").dim().italic());
                print_markdown("Thinking was redacted", theme);
            }
            MessageContent::SystemNotification(notification) => {
                match notification.notification_type {
                    SystemNotificationType::ThinkingMessage
                    | SystemNotificationType::ProgressMessage => {
                        show_thinking();
                        set_thinking_message(&notification.msg);
                    }
                    SystemNotificationType::InlineMessage => {
                        hide_thinking();
                        println!("\n{} {}", style("·").dim(), &notification.msg);
                    }
                    SystemNotificationType::CreditsExhausted => {
                        render_credits_exhausted_notification(notification);
                    }
                }
            }
            _ => {
                eprintln!("WARNING: Message content type could not be rendered");
            }
        }
    }

    let _ = std::io::stdout().flush();
}

/// Render a streaming message, using a buffer to accumulate text content
/// and only render when markdown constructs are complete.
pub fn render_message_streaming(
    message: &Message,
    buffer: &mut MarkdownBuffer,
    thinking_header_shown: &mut bool,
    debug: bool,
) {
    if !message.is_user_visible() {
        return;
    }
    let message = message.user_visible_content();
    let theme = get_theme();

    for content in &message.content {
        if !matches!(content, MessageContent::Thinking(_)) {
            if *thinking_header_shown {
                println!();
            }
            *thinking_header_shown = false;
        }

        match content {
            MessageContent::Text(text) => {
                if let Some(safe_content) = buffer.push(&text.text) {
                    print_markdown(&safe_content, theme);
                }
            }
            MessageContent::ToolRequest(req) => {
                flush_markdown_buffer(buffer, theme);
                render_tool_request(req, theme, debug);
            }
            MessageContent::ToolResponse(resp) => {
                flush_markdown_buffer(buffer, theme);
                render_tool_response(resp, debug);
                begin_answer();
            }
            MessageContent::ActionRequired(action) => {
                flush_markdown_buffer(buffer, theme);
                match &action.data {
                    ActionRequiredData::ToolConfirmation { tool_name, .. } => {
                        println!("action_required(tool_confirmation): {}", tool_name)
                    }
                    ActionRequiredData::Elicitation { message, .. } => {
                        println!("action_required(elicitation): {}", message)
                    }
                    ActionRequiredData::ElicitationResponse { id, .. } => {
                        println!("action_required(elicitation_response): {}", id)
                    }
                    ActionRequiredData::ToolConfirmationResponse { id, .. } => {
                        println!("action_required(tool_confirmation_response): {}", id)
                    }
                }
            }
            MessageContent::Image(image) => {
                flush_markdown_buffer(buffer, theme);
                println!("Image: [data: {}, type: {}]", image.data, image.mime_type);
            }
            MessageContent::Thinking(t) => {
                render_thinking_streaming(&t.thinking, buffer, thinking_header_shown, theme);
            }
            MessageContent::RedactedThinking(_) => {
                flush_markdown_buffer(buffer, theme);
                println!("\n{}", style("Thinking:").dim().italic());
                print_markdown("Thinking was redacted", theme);
            }
            MessageContent::SystemNotification(notification) => {
                match notification.notification_type {
                    SystemNotificationType::ThinkingMessage
                    | SystemNotificationType::ProgressMessage => {
                        show_thinking();
                        set_thinking_message(&notification.msg);
                    }
                    SystemNotificationType::InlineMessage => {
                        flush_markdown_buffer(buffer, theme);
                        hide_thinking();
                        println!("\n{} {}", style("·").dim(), &notification.msg);
                    }
                    SystemNotificationType::CreditsExhausted => {
                        flush_markdown_buffer(buffer, theme);
                        render_credits_exhausted_notification(notification);
                    }
                }
            }
            _ => {
                flush_markdown_buffer(buffer, theme);
                eprintln!("WARNING: Message content type could not be rendered");
            }
        }
    }

    let _ = std::io::stdout().flush();
}

fn render_credits_exhausted_notification(notification: &SystemNotificationContent) {
    hide_thinking();
    println!("\n{} {}", warning("warning:").bold(), &notification.msg);

    if let Some(url) = notification
        .data
        .as_ref()
        .and_then(|d| d.get("top_up_url"))
        .and_then(|v| v.as_str())
    {
        println!("{} {}", style("top up:").dim(), accent(url));
    }
}

pub fn get_credits_top_up_url(message: &Message) -> Option<String> {
    message.content.iter().find_map(|content| {
        let MessageContent::SystemNotification(notification) = content else {
            return None;
        };
        if notification.notification_type != SystemNotificationType::CreditsExhausted {
            return None;
        }
        notification
            .data
            .as_ref()
            .and_then(|d| d.get("top_up_url"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    })
}

pub fn flush_markdown_buffer(buffer: &mut MarkdownBuffer, theme: Theme) {
    let remaining = buffer.flush();
    if !remaining.is_empty() {
        print_markdown(&remaining, theme);
    }
}

pub fn flush_markdown_buffer_current_theme(buffer: &mut MarkdownBuffer) {
    flush_markdown_buffer(buffer, get_theme());
}

pub fn render_text(text: &str, color: Option<Color>, dim: bool) {
    render_text_no_newlines(format!("\n{}\n\n", text).as_str(), color, dim);
}

pub fn render_text_no_newlines(text: &str, color: Option<Color>, dim: bool) {
    if !std::io::stdout().is_terminal() {
        println!("{}", text);
        return;
    }
    let mut styled_text = style(text);
    if dim {
        styled_text = styled_text.dim();
    }
    if let Some(color) = color {
        styled_text = styled_text.fg(color);
    }
    print!("{}", styled_text);
}

pub fn render_enter_plan_mode() {
    open_block();
    println!(
        "{} {}",
        accent("Entering plan mode.").bold(),
        style("You can provide instructions to create a plan and then act on it. To exit early, type /endplan")
            .dim()
    );
}

pub fn render_act_on_plan() {
    open_block();
    println!(
        "{}",
        accent("Exiting plan mode and acting on the above plan").bold(),
    );
}

pub fn render_exit_plan_mode() {
    open_block();
    println!("{}", accent("Exiting plan mode.").bold());
}

pub fn render_interrupted() {
    open_block();
    println!(
        "{}",
        style("Interrupted — the request above was dropped").dim()
    );
}

pub fn render_plan_interrupted() {
    open_block();
    println!(
        "{}",
        style("Plan request interrupted - still in plan mode, /endplan to exit").dim()
    );
}

pub fn render_plan_kept() {
    open_block();
    println!(
        "{}",
        style("Plan kept in the conversation - still in plan mode, /endplan to exit").dim()
    );
}

pub fn goose_mode_message(text: &str) {
    println!("\n{} {}", accent("mode:"), text);
}

fn should_show_thinking() -> bool {
    Config::global()
        .get_param::<bool>("GOOSE_CLI_SHOW_THINKING")
        .unwrap_or(false)
        && std::io::stdout().is_terminal()
}

fn render_thinking(text: &str, theme: Theme) {
    if should_show_thinking() {
        // the marker belongs to the answer, not to the reasoning before it
        let marker = take_marker();
        open_block();
        println!("{}", style("Thinking:").dim().italic());
        print_markdown(text, theme);
        if let Some(marker) = marker {
            set_marker(marker);
        }
    }
}

fn render_thinking_streaming(
    text: &str,
    buffer: &mut MarkdownBuffer,
    header_shown: &mut bool,
    theme: Theme,
) {
    if should_show_thinking() {
        flush_markdown_buffer(buffer, theme);
        if !*header_shown {
            open_block();
            println!("{}", style("Thinking:").dim().italic());
            *header_shown = true;
        }
        print!("{}", style(text).dim());
        let _ = std::io::stdout().flush();
    }
}

fn render_tool_request(req: &ToolRequest, theme: Theme, debug: bool) {
    match &req.tool_call {
        Ok(call) => {
            if answers_the_turn(&call.name) {
                remember_full_result(&req.id);
            }

            // a turn makes a dozen calls and is read to see what the agent is up
            // to, so by default a call takes one line and its output a second
            // one; /r brings back the whole of both
            let name_is_title = req.was_executed_externally();
            if !debug && !get_show_full_tool_output() {
                return render_tool_call_line(call, name_is_title, debug);
            }

            render_verbose_tool_request(call, name_is_title, debug)
        }
        Err(e) => print_markdown(&e.to_string(), theme),
    }
}

/// Whether the result of a call is the answer the turn was after, rather than a
/// step towards it. What a delegate reports back is the whole point of running
/// it, so it is never traded for a line saying how long it was.
fn answers_the_turn(tool_name: &str) -> bool {
    ToolNameParts::from(tool_name).tool_name == "load"
}

fn remember_full_result(id: &str) {
    FULL_RESULT_IDS.with(|ids| {
        let mut ids = ids.borrow_mut();
        ids.push_back(id.to_string());
        while ids.len() > FULL_RESULT_MEMORY {
            ids.pop_front();
        }
    });
}

fn result_is_the_answer(id: &str) -> bool {
    FULL_RESULT_IDS.with(|ids| ids.borrow().iter().any(|known| known == id))
}

/// A call the way `/r` shows it: a header, then every parameter on a line of its
/// own.
fn render_verbose_tool_request(call: &CallToolRequestParams, name_is_title: bool, debug: bool) {
    match call.name.to_string().as_str() {
        name if is_shell_tool_name(name) => render_shell_request(call, debug),
        name if is_file_tool_name(name) => render_text_editor_request(call, debug),
        "execute_typescript" | "execute_code" => render_execute_code_request(call, debug),
        "delegate" => render_delegate_request(call, debug),
        "subagent" => render_delegate_request(call, debug),
        "todo__write" => render_todo_request(call, debug),
        _ => render_default_request(call, name_is_title, debug),
    }
}

/// A call the way it is shown by default: what was called and with what, on one
/// line. The list of calls a code-mode run plans out keeps its own shape, being
/// a list of calls rather than one of them.
fn render_tool_call_line(call: &CallToolRequestParams, name_is_title: bool, debug: bool) {
    if matches!(
        call.name.to_string().as_str(),
        "execute_typescript" | "execute_code"
    ) {
        return render_execute_code_request(call, debug);
    }

    open_tool_run();
    let head = tool_call_head(display_parts(&call.name, name_is_title));
    let room = terminal_width().saturating_sub(measure_text_width(&head) + 2);
    match summarize_params(call.arguments.as_ref(), room, debug) {
        Some(summary) => println!("{}  {}", head, style(summary).dim()),
        None => println!("{}", head),
    }
}

/// How a call is named on screen. A name an agent ran for us is its own title,
/// so a command like `ls src/__pycache__` keeps its dunders instead of reading
/// as an `extension__tool` pair.
fn display_parts(tool_name: &str, name_is_title: bool) -> ToolNameParts<'_> {
    match name_is_title {
        true => ToolNameParts {
            extension_name: None,
            tool_name,
        },
        false => ToolNameParts::from(tool_name),
    }
}

fn tool_call_head(parts: ToolNameParts<'_>) -> String {
    match parts.extension_name {
        Some(extension_name) => format!(
            "  {} {} {}",
            style("▸").dim(),
            style(parts.tool_name).dim(),
            style(extension_display_name(extension_name))
                .magenta()
                .dim(),
        ),
        None => format!("  {} {}", style("▸").dim(), style(parts.tool_name).dim()),
    }
}

fn is_visible_to_user(
    annotations: Option<&Annotations>,
    min_priority: f32,
    is_error: bool,
) -> bool {
    if let Some(audience) = annotations.and_then(|a| a.audience.as_ref()) {
        if !audience.contains(&Role::User) {
            return false;
        }
    }

    if is_error {
        return true;
    }

    annotations
        .and_then(|a| a.priority)
        .unwrap_or(DEFAULT_MIN_PRIORITY)
        >= min_priority
}

fn render_tool_response(resp: &ToolResponse, debug: bool) {
    let config = Config::global();

    match &resp.tool_result {
        Ok(result) => {
            let min_priority = config
                .get_param::<f32>("GOOSE_CLI_MIN_PRIORITY")
                .ok()
                .unwrap_or(DEFAULT_MIN_PRIORITY);
            let is_error = result.is_error.unwrap_or(false);

            for content in &result.content {
                let annotations = match content {
                    rmcp::model::ContentBlock::Text(t) => t.annotations.as_ref(),
                    rmcp::model::ContentBlock::Image(i) => i.annotations.as_ref(),
                    rmcp::model::ContentBlock::Audio(a) => a.annotations.as_ref(),
                    rmcp::model::ContentBlock::Resource(r) => r.annotations.as_ref(),
                    rmcp::model::ContentBlock::ResourceLink(r) => r.annotations.as_ref(),
                    _ => None,
                };
                if !is_visible_to_user(annotations, min_priority, is_error) {
                    continue;
                }

                if debug {
                    println!("{:#?}", content);
                } else if let Some(text) = content.as_text() {
                    print_tool_output(&text.text, is_error, result_is_the_answer(&resp.id));
                }
            }
        }
        Err(e) => {
            open_tool_run();
            println!("    {}", style(e.to_string()).red());
        }
    }
}

/// Tool output is untrusted text: an escape sequence in it would otherwise
/// repaint the screen, rewrite the window title or reach the clipboard.
pub(super) fn sanitize_terminal_line(line: &str) -> String {
    strip_str(line)
        .flat_map(str::chars)
        .filter(|character| *character == '\t' || !character.is_control())
        .collect()
}

fn print_tool_output(text: &str, is_error: bool, is_the_answer: bool) {
    if text.is_empty() {
        return;
    }
    if !std::io::stdout().is_terminal() {
        print!("{}", text);
        return;
    }

    // the output belongs to the call above it, so in the compact view it joins
    // the run that call opened instead of starting a block of its own
    let compact = !get_show_full_tool_output();
    if compact {
        open_tool_run();
    } else {
        open_block();
    }

    let lines: Vec<&str> = text.lines().collect();
    // an error is what the output is read for, and an answer is what the turn
    // was after; neither is ever traded for a line about its length
    if compact && !is_error && !is_the_answer && lines.len() > COMPACT_OUTPUT_LINES {
        println!("    {}", style(collapsed_output_note(lines.len())).dim());
        return;
    }

    let max_lines = if compact && !is_the_answer && is_error {
        20
    } else {
        usize::MAX
    };
    let paint = |line: &str| {
        let styled = style(sanitize_terminal_line(line));
        if is_error {
            styled.red()
        } else {
            styled.dim()
        }
    };
    if lines.len() <= max_lines {
        for line in &lines {
            println!("    {}", paint(line));
        }
    } else {
        let head = max_lines / 2;
        let tail = max_lines - head;
        for line in &lines[..head] {
            println!("    {}", paint(line));
        }
        println!(
            "    {}",
            style(format!(
                "... ({} lines hidden, /r to show all)",
                lines.len() - head - tail
            ))
            .dim()
            .italic()
        );
        for line in &lines[lines.len() - tail..] {
            println!("    {}", paint(line));
        }
    }
}

/// What stands in for output that was folded away. The way to unfold it is
/// spelled out once a session, since after that it is the count that carries
/// the news and the advice would just be noise.
fn collapsed_output_note(lines: usize) -> String {
    let told = OUTPUT_HINT_SHOWN.with(|shown| shown.replace(true));
    match told {
        true => format!("{lines} lines"),
        false => format!("{lines} lines · /r shows full output"),
    }
}

fn is_shell_tool_name(name: &str) -> bool {
    matches!(name, "shell")
}

fn is_file_tool_name(name: &str) -> bool {
    matches!(name, "write" | "edit")
}

pub fn render_error(message: &str) {
    println!("\n  {} {}\n", danger("error:").bold(), message);
}

/// Something changed under the session rather than went wrong.
pub fn render_note(message: &str) {
    println!("\n  {} {}\n", warning("note:").bold(), message);
}

pub fn render_prompts(prompts: &HashMap<String, Vec<String>>) {
    println!();
    for (extension, prompts) in prompts {
        println!(" {}", accent(extension));
        for prompt in prompts {
            println!("  - {}", style(prompt).cyan());
        }
    }
    println!();
}

pub fn render_prompt_info(info: &PromptInfo) {
    println!();
    if let Some(ext) = &info.extension {
        println!(" {}: {}", accent("Extension"), ext);
    }
    println!(" Prompt: {}", style(&info.name).cyan().bold());
    if let Some(desc) = &info.description {
        println!("\n {}", desc);
    }
    render_arguments(info);
    println!();
}

fn render_arguments(info: &PromptInfo) {
    if let Some(args) = &info.arguments {
        println!("\n Arguments:");
        for arg in args {
            let required = arg.required.unwrap_or(false);
            let req_str = if required {
                style("(required)").bold()
            } else {
                style("(optional)").dim()
            };

            println!(
                "  {} {} {}",
                accent(&arg.name),
                req_str,
                arg.description.as_deref().unwrap_or("")
            );
        }
    }
}

pub fn render_mode_usage(current: GooseMode) {
    println!();
    println!("  {} {}", accent("mode:"), current);
    for &name in GooseMode::VARIANTS {
        let description = GooseMode::from_str(name)
            .ok()
            .and_then(|mode| mode.get_message())
            .unwrap_or_default();
        println!("    {:<14} {}", name, style(description).dim());
    }
    println!();
    println!("{}", style("  usage: /mode <name>").dim());
    println!();
}

pub fn render_extension_error(name: &str, error: &str) {
    println!();
    println!("  {} to add extension {}", danger("failed"), danger(name));
    println!();
    println!("{}", style(error).dim());
    println!();
}

fn render_text_editor_request(call: &CallToolRequestParams, debug: bool) {
    print_tool_header(call);

    if let Some(args) = &call.arguments {
        if let Some(Value::String(path)) = args.get("path") {
            println!(
                "    {} {}",
                style("path").dim(),
                style(shorten_path(path, debug)).dim()
            );
        }

        if let Some(args) = &call.arguments {
            let mut other_args = serde_json::Map::new();
            for (k, v) in args {
                if k != "path" {
                    other_args.insert(k.clone(), v.clone());
                }
            }
            if !other_args.is_empty() {
                print_params(&Some(other_args), 1, debug);
            }
        }
    }
}

fn render_shell_request(call: &CallToolRequestParams, debug: bool) {
    print_tool_header(call);
    print_params(&call.arguments, 1, debug);
}

fn render_execute_code_request(call: &CallToolRequestParams, debug: bool) {
    let tool_graph = call
        .arguments
        .as_ref()
        .and_then(|args| args.get("tool_graph"))
        .and_then(Value::as_array)
        .filter(|arr| !arr.is_empty());

    let Some(tool_graph) = tool_graph else {
        return render_default_request(call, false, debug);
    };

    let count = tool_graph.len();
    let plural = if count == 1 { "" } else { "s" };
    println!();
    println!(
        "  {} {} {} tool call{}",
        style("▸").dim(),
        style("execute").dim(),
        style(count).dim(),
        plural,
    );

    for (i, node) in tool_graph.iter().filter_map(Value::as_object).enumerate() {
        let tool = node
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let desc = node
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let deps: Vec<_> = node
            .get("depends_on")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|d| (d + 1).to_string())
            .collect();
        let deps_str = if deps.is_empty() {
            String::new()
        } else {
            format!(" (uses {})", deps.join(", "))
        };
        println!(
            "    {}. {} {}{}",
            style(i + 1).dim(),
            style(tool).dim(),
            style(desc).dim(),
            style(deps_str).dim()
        );
    }

    let code = call
        .arguments
        .as_ref()
        .and_then(|args| args.get("code"))
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty());
    if code.is_some_and(|_| debug) {
        println!("{}", code.unwrap_or_default());
    }
}

fn render_delegate_request(call: &CallToolRequestParams, debug: bool) {
    print_tool_header(call);

    if let Some(args) = &call.arguments {
        if let Some(Value::String(source)) = args.get("source") {
            println!("    {} {}", style("source").dim(), style(source).dim());
        }

        if let Some(Value::String(instructions)) = args.get("instructions") {
            let display = if instructions.len() > 100 && !debug {
                safe_truncate(instructions, 100)
            } else {
                instructions.clone()
            };
            println!(
                "    {} {}",
                style("instructions").dim(),
                style(display).dim()
            );
        }

        if let Some(Value::Object(params)) = args.get("parameters") {
            println!("    {}:", style("parameters").dim());
            print_params(&Some(params.clone()), 2, debug);
        }

        let skip_keys = ["source", "instructions", "parameters"];
        let mut other_args = serde_json::Map::new();
        for (k, v) in args {
            if !skip_keys.contains(&k.as_str()) {
                other_args.insert(k.clone(), v.clone());
            }
        }
        if !other_args.is_empty() {
            print_params(&Some(other_args), 1, debug);
        }
    }
}

fn render_todo_request(call: &CallToolRequestParams, _debug: bool) {
    print_tool_header(call);

    if let Some(args) = &call.arguments {
        if let Some(Value::String(content)) = args.get("content") {
            println!("    {} {}", style("content").dim(), style(content).dim());
        }
    }
}

fn render_default_request(call: &CallToolRequestParams, name_is_title: bool, debug: bool) {
    print_tool_header_parts(display_parts(&call.name, name_is_title));
    print_params(&call.arguments, 1, debug);
}

fn extension_display_name(name: &str) -> &str {
    match name {
        "code_execution" => "Code Mode",
        _ => name,
    }
}

/// The tool of a subagent call, without the extension a plain call would not
/// name either.
fn subagent_tool_name(tool_name: &str) -> String {
    let parts = ToolNameParts::from(tool_name);

    match parts.extension_name {
        Some(extension_name) => format!(
            "{} | {}",
            parts.tool_name,
            extension_display_name(extension_name)
        ),
        None => parts.tool_name.to_string(),
    }
}

pub fn format_subagent_tool_call_message(subagent_id: &str, tool_name: &str) -> String {
    let short_id = subagent_id.rsplit('_').next().unwrap_or(subagent_id);
    format!("[subagent:{}] {}", short_id, subagent_tool_name(tool_name))
}

pub fn render_subagent_tool_call(
    subagent_id: &str,
    label: Option<&str>,
    tool_name: &str,
    arguments: Option<&JsonObject>,
    debug: bool,
) {
    if tool_name == "code_execution__execute_typescript" {
        let tool_graph = arguments
            .and_then(|args| args.get("tool_graph"))
            .and_then(Value::as_array)
            .filter(|arr| !arr.is_empty());
        if let Some(tool_graph) = tool_graph {
            return render_subagent_tool_graph(subagent_id, label, tool_graph);
        }
    }

    let name = subagent_label(subagent_id, label);
    open_tool_run();

    let head = format!(
        "  {} {} {}",
        style("▸").dim(),
        style(&name).dim(),
        style(subagent_tool_name(tool_name)).dim(),
    );

    // a delegate calls tools by the dozen, and the stream is read to see what it
    // is up to rather than with what exactly, so a call takes one line
    let room = terminal_width().saturating_sub(measure_text_width(&head) + 2);
    match summarize_params(arguments, room, debug) {
        Some(summary) => println!("{}  {}", head, style(summary).dim()),
        None => println!("{}", head),
    }
}

/// Folds the parameters of a call onto its line, keeping to the room left over.
fn summarize_params(arguments: Option<&JsonObject>, room: usize, debug: bool) -> Option<String> {
    let arguments = arguments?;

    let mut parts = Vec::new();
    for (key, value) in arguments {
        let text = match value {
            Value::Null => continue,
            // a path is the one parameter that is routinely longer than the line
            // it goes on, and its beginning is the part that says nothing
            Value::String(text) if key == "path" => shorten_path(text, debug),
            Value::String(text) => fold_lines(text),
            other => other.to_string(),
        };
        if text.is_empty() {
            continue;
        }
        parts.push(format!("{key} {text}"));
    }

    if parts.is_empty() {
        return None;
    }

    let summary = parts.join("  ");
    if debug || summary.chars().count() <= room {
        return Some(summary);
    }
    Some(safe_truncate(&summary, room))
}

fn render_subagent_tool_graph(subagent_id: &str, label: Option<&str>, tool_graph: &[Value]) {
    let name = subagent_label(subagent_id, label);
    let count = tool_graph.len();
    let plural = if count == 1 { "" } else { "s" };
    open_tool_run();
    println!(
        "  {} {} {} {} tool call{}",
        style("▸").dim(),
        style(&name).dim(),
        style("execute_typescript").dim(),
        style(count).dim(),
        plural,
    );

    for (i, node) in tool_graph.iter().filter_map(Value::as_object).enumerate() {
        let tool = node
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let desc = node
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let deps: Vec<_> = node
            .get("depends_on")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|d| (d + 1).to_string())
            .collect();
        let deps_str = if deps.is_empty() {
            String::new()
        } else {
            format!(" (uses {})", deps.join(", "))
        };
        println!(
            "    {}. {} {}{}",
            style(i + 1).dim(),
            style(tool).dim(),
            style(desc).dim(),
            style(deps_str).dim()
        );
    }
}

// Helper functions

fn print_tool_header(call: &CallToolRequestParams) {
    print_tool_header_parts(ToolNameParts::from(call.name.as_ref()));
}

fn print_tool_header_parts(parts: ToolNameParts<'_>) {
    let tool_header = match parts.extension_name {
        Some(extension_name) => format!(
            "  {} {} {}",
            style("▸").dim(),
            style(parts.tool_name).dim(),
            style(extension_display_name(extension_name))
                .magenta()
                .dim(),
        ),
        None => format!("  {} {}", style("▸").dim(), style(parts.tool_name).dim()),
    };
    open_block();
    println!("  {}", style("─".repeat(40)).dim());
    println!("{}", tool_header);
}

// Respect NO_COLOR, as https://crates.io/crates/console already does
pub fn env_no_color() -> bool {
    // if NO_COLOR is defined at all disable colors
    std::env::var_os("NO_COLOR").is_none()
}

fn print_markdown(content: &str, theme: Theme) {
    if std::io::stdout().is_terminal() {
        if let Some((before, table, after)) = extract_markdown_table(content) {
            if !before.is_empty() {
                print_markdown_raw(&before, theme);
            }
            print_table(&table, theme);
            if !after.is_empty() {
                print_markdown(after, theme);
            }
        } else {
            print_markdown_raw(content, theme);
        }
    } else {
        print!("{}", content);
    }
}

/// Renders markdown content using bat (no table processing)
fn print_markdown_raw(content: &str, theme: Theme) {
    let marker = take_marker();

    // models like to open with a blank line or two, which would add to the
    // spacing the session lays out; only the block that opens a turn is trimmed,
    // because later chunks carry the paragraph breaks of the answer itself
    let content = match marker {
        Some(_) => content.trim_start_matches('\n'),
        None => content,
    };

    if content.trim().is_empty() {
        // nothing to mark yet, so the marker waits for the chunk that has text
        if let Some(marker) = marker {
            set_marker(marker);
        }
        return;
    }

    // a marked block carries its own empty line, so the run of tool calls above
    // it ends without one being printed here
    end_tool_run();

    let width = terminal_width().saturating_sub(REPLY_INDENT.len());
    let wrapped = wrap_markdown_source(content, width);

    // bat only wraps character by character, so the text arrives already broken
    // into lines and is caught here instead of going to stdout, which is what
    // lets every line of it be moved in by the same amount
    let mut rendered = String::new();
    bat::PrettyPrinter::new()
        .input(bat::Input::from_bytes(wrapped.as_bytes()))
        .theme(theme.as_str())
        .colored_output(env_no_color())
        .language("Markdown")
        .wrapping_mode(WrappingMode::NoWrapping(true))
        .term_width(width)
        .print_with_writer(Some(&mut rendered))
        .unwrap();

    if rendered.is_empty() {
        return;
    }

    print!("{}", indent_block(&rendered, marker.as_deref()));
}

/// Breaks prose at the given width so that the rendered block can be moved in
/// as a whole. Fenced code and table rows are left alone: they are meant to be
/// read as written, and the terminal still wraps them if they do not fit.
fn wrap_markdown_source(content: &str, width: usize) -> String {
    if width == 0 {
        return content.to_string();
    }

    let mut out = String::with_capacity(content.len());
    let mut in_fence = false;

    for line in content.split_inclusive('\n') {
        let (text, ending) = match line.strip_suffix('\n') {
            Some(text) => (text, "\n"),
            None => (line, ""),
        };
        let trimmed = text.trim_start();
        let is_fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");

        if is_fence {
            in_fence = !in_fence;
        }

        let untouched = in_fence
            || is_fence
            || trimmed.starts_with('|')
            || text.starts_with("    ")
            || text.chars().count() <= width;

        if untouched {
            out.push_str(text);
            out.push_str(ending);
            continue;
        }

        let indent: String = text.chars().take_while(|c| c.is_whitespace()).collect();
        let room = width.saturating_sub(indent.chars().count()).max(1);
        for (index, part) in wrap_words(trimmed, room).iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            out.push_str(&indent);
            out.push_str(part);
        }
        out.push_str(ending);
    }

    out
}

/// Puts a rendered block behind the reply indent, with the marker of the
/// speaker in the columns the indent frees up.
fn indent_block(rendered: &str, marker: Option<&str>) -> String {
    let mut out = String::with_capacity(rendered.len());
    let mut first = true;

    // a marked block opens a turn or resumes one after a tool call, and every
    // block in the session is separated from the one above by a single line
    if marker.is_some() {
        out.push('\n');
    }

    for line in rendered.split_inclusive('\n') {
        let bare = line.strip_suffix('\n').unwrap_or(line);
        if !bare.is_empty() {
            match marker.filter(|_| first) {
                Some(marker) => out.push_str(&format!("{marker} ")),
                None => out.push_str(REPLY_INDENT),
            }
            first = false;
        }
        out.push_str(line);
    }

    out
}

fn extract_markdown_table(content: &str) -> Option<(String, Vec<&str>, &str)> {
    let lines: Vec<&str> = content.lines().collect();

    // Track newline positions for safe slicing later
    let newline_indices: Vec<usize> = content
        .bytes()
        .enumerate()
        .filter_map(|(i, b)| if b == b'\n' { Some(i) } else { None })
        .collect();

    // Skip tables inside code blocks
    let mut in_code_block = false;
    let mut table_start = None;
    let mut table_end = None;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            continue;
        }

        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            if table_start.is_none() {
                table_start = Some(i);
            }
            table_end = Some(i);
        } else if table_start.is_some() {
            break;
        }
    }

    let start = table_start?;
    let end = table_end?;

    // Need at least header + separator (2 rows minimum)
    if end < start + 1 {
        return None;
    }

    // Require separator to be the second row with proper format
    let separator_line = lines.get(start + 1)?;
    let is_valid_separator = separator_line.trim().starts_with('|')
        && separator_line.trim().ends_with('|')
        && separator_line
            .trim()
            .trim_matches('|')
            .split('|')
            .all(|cell| {
                let t = cell.trim();
                !t.is_empty() && t.chars().all(|c| c == '-' || c == ':' || c == ' ')
            });

    if !is_valid_separator {
        return None;
    }

    let before = lines[..start].join("\n");
    let before = if before.is_empty() {
        before
    } else {
        before + "\n"
    };
    let table = lines[start..=end].to_vec();

    let after = if end + 1 >= lines.len() {
        ""
    } else if let Some(&newline_pos) = newline_indices.get(end) {
        content.get(newline_pos + 1..).unwrap_or("")
    } else {
        ""
    };

    Some((before, table, after))
}

/// Width of the terminal, or a conservative default when it cannot be read.
/// comfy-table is built without its `tty` feature and never measures it itself.
pub fn terminal_width() -> usize {
    Term::stdout()
        .size_checked()
        .map(|(_height, width)| width as usize)
        .unwrap_or(80)
}

fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut lines = Vec::new();
    let mut start = 0;

    while start < words.len() {
        let mut end = start;
        let mut taken = 0;

        while end < words.len() {
            let extra = words[end].chars().count() + usize::from(end > start);
            if end > start && taken + extra > width {
                break;
            }
            taken += extra;
            end += 1;
        }

        // a break must not hand the next line to markdown punctuation, or the
        // tail of a paragraph is read as a list, a heading or a quote
        while end > start + 1 && end < words.len() && starts_like_markdown(words[end]) {
            end -= 1;
        }

        lines.push(words[start..end].join(" "));
        start = end;
    }

    lines
}

fn starts_like_markdown(word: &str) -> bool {
    if matches!(word, "-" | "*" | "+" | "1.") || word.starts_with('>') {
        return true;
    }
    !word.is_empty() && word.chars().all(|c| c == '#') && word.chars().count() <= 6
}

fn print_table(table_lines: &[&str], theme: Theme) {
    use comfy_table::{presets, Cell, CellAlignment, ContentArrangement, Table};

    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_width(terminal_width() as u16);

    table.load_preset(presets::ASCII_MARKDOWN);

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut alignments: Vec<CellAlignment> = Vec::new();
    let mut separator_idx = None;

    for (i, line) in table_lines.iter().enumerate() {
        let cells: Vec<String> = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(|s| s.trim().to_string())
            .collect();

        let is_separator = cells.iter().all(|c| {
            let t = c.trim();
            t.chars().all(|ch| ch == '-' || ch == ':') && t.contains('-')
        });
        if is_separator {
            separator_idx = Some(i);
            alignments = cells
                .iter()
                .map(|c| {
                    let t = c.trim();
                    if t.starts_with(':') && t.ends_with(':') {
                        CellAlignment::Center
                    } else if t.ends_with(':') {
                        CellAlignment::Right
                    } else {
                        CellAlignment::Left
                    }
                })
                .collect();
        } else {
            rows.push(cells);
        }
    }

    if separator_idx.is_none() && !rows.is_empty() {
        alignments = vec![CellAlignment::Left; rows[0].len()];
    }

    if let Some(header) = rows.first() {
        let header_cells: Vec<Cell> = header
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let cell = Cell::new(text);
                if let Some(align) = alignments.get(i) {
                    cell.set_alignment(*align)
                } else {
                    cell
                }
            })
            .collect();
        table.set_header(header_cells);
    }

    for row in rows.iter().skip(1) {
        let cells: Vec<Cell> = row
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let cell = Cell::new(text);
                if let Some(align) = alignments.get(i) {
                    cell.set_alignment(*align)
                } else {
                    cell
                }
            })
            .collect();
        table.add_row(cells);
    }

    let table_str = table.to_string();
    print_markdown_raw(&table_str, theme);
}

const INDENT: &str = "    ";

/// A parameter takes one line, so a value carrying newlines of its own would
/// break out of the indent everything else is printed at. Folded into a line
/// when the value is only being previewed, and moved to the column it starts at
/// when the whole of it was asked for.
fn fit_value(
    text: &str,
    max_width: Option<usize>,
    reserve_width: usize,
    show_full: bool,
) -> String {
    if show_full {
        return text.replace('\n', &format!("\n{}", " ".repeat(reserve_width)));
    }

    let folded = fold_lines(text);
    match max_width {
        Some(width) if folded.chars().count() > width => safe_truncate(&folded, width),
        _ => folded,
    }
}

/// Puts a value on one line. Whitespace collapses only around the line breaks
/// that are removed, so a command keeps the spacing it was written with.
fn fold_lines(text: &str) -> String {
    if !text.contains('\n') {
        return text.to_string();
    }

    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn print_value_with_prefix(prefix: &String, value: &Value, debug: bool) {
    let prefix_width = measure_text_width(prefix.as_str());
    print!("{}", prefix);
    print_value(value, debug, prefix_width)
}

fn print_value(value: &Value, debug: bool, reserve_width: usize) {
    let max_width = Term::stdout()
        .size_checked()
        .map(|(_h, w)| (w as usize).saturating_sub(reserve_width));
    let show_full = get_show_full_tool_output();
    let formatted = match value {
        Value::String(s) => {
            style(fit_value(s, max_width, reserve_width, debug || show_full)).green()
        }
        Value::Number(n) => style(n.to_string()).yellow(),
        Value::Bool(b) => style(b.to_string()).yellow(),
        Value::Null => style("null".to_string()).dim(),
        _ => unreachable!(),
    };
    println!("{}", formatted);
}

fn print_params(value: &Option<JsonObject>, depth: usize, debug: bool) {
    let indent = INDENT.repeat(depth);

    if let Some(json_object) = value {
        for (key, val) in json_object.iter() {
            match val {
                Value::Object(obj) => {
                    println!("{}{}:", indent, style(key).dim());
                    print_params(&Some(obj.clone()), depth + 1, debug);
                }
                Value::Array(arr) => {
                    // Check if all items are simple values (not objects or arrays)
                    let all_simple = arr.iter().all(|item| {
                        matches!(
                            item,
                            Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
                        )
                    });

                    if all_simple {
                        // Render inline for simple arrays, truncation will be handled by print_value if needed
                        let values: Vec<String> = arr
                            .iter()
                            .map(|item| match item {
                                Value::String(s) => s.clone(),
                                Value::Number(n) => n.to_string(),
                                Value::Bool(b) => b.to_string(),
                                Value::Null => "null".to_string(),
                                _ => unreachable!(),
                            })
                            .collect();
                        let joined_values = values.join(", ");
                        print_value_with_prefix(
                            &format!("{}{}: ", indent, style(key).dim()),
                            &Value::String(joined_values),
                            debug,
                        );
                    } else {
                        // Use the original multi-line format for complex arrays
                        println!("{}{}:", indent, style(key).dim());
                        for item in arr.iter() {
                            if let Value::Object(obj) = item {
                                println!("{}{}- ", indent, INDENT);
                                print_params(&Some(obj.clone()), depth + 2, debug);
                            } else {
                                println!("{}{}- {}", indent, INDENT, item);
                            }
                        }
                    }
                }
                _ => {
                    print_value_with_prefix(
                        &format!("{}{}: ", indent, style(key).dim()),
                        val,
                        debug,
                    );
                }
            }
        }
    }
}

fn shorten_path(path: &str, debug: bool) -> String {
    // In debug mode, return the full path
    if debug {
        return path.to_string();
    }

    let path = Path::new(path);

    // First try to convert to ~ if it's in home directory
    let home = etcetera::home_dir().ok();
    let path_str = if let Some(home) = home {
        if let Ok(stripped) = path.strip_prefix(home) {
            format!("~/{}", stripped.display())
        } else {
            path.display().to_string()
        }
    } else {
        path.display().to_string()
    };

    // If path is already short enough, return as is
    if path_str.len() <= 60 {
        return path_str;
    }

    let parts: Vec<_> = path_str.split('/').collect();

    // If we have 3 or fewer parts, return as is
    if parts.len() <= 3 {
        return path_str;
    }

    // Keep the first component (empty string before root / or ~) and last two components intact
    let mut shortened = vec![parts[0].to_string()];

    // Shorten middle components to their first letter
    for component in &parts[1..parts.len() - 2] {
        if !component.is_empty() {
            shortened.push(component.chars().next().unwrap_or('?').to_string());
        }
    }

    // Add the last two components
    shortened.push(parts[parts.len() - 2].to_string());
    shortened.push(parts[parts.len() - 1].to_string());

    shortened.join("/")
}

pub fn display_banner(banners: &[String]) {
    for banner in banners {
        for line in banner.lines() {
            println!("{}", line);
        }
    }
}

pub fn display_session_info(
    resume: bool,
    provider: &str,
    model: &str,
    session_id: &Option<String>,
) {
    set_terminal_title();

    let status = if resume {
        "resuming"
    } else if session_id.is_none() {
        "ephemeral"
    } else {
        "new session"
    };

    let model_display = model.to_string();

    let cwd_display = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let mode = Config::global().get_goose_mode().unwrap_or_default();

    // ASCII art with session info on the right
    println!();
    println!(
        "  {}  {} {} {} {} {} {} {}",
        style(r" (\(\ ").white(),
        style("●").green(),
        style(status).dim(),
        style("·").dim(),
        style(provider).dim(),
        style(&model_display).cyan(),
        style("·").dim(),
        style(mode.to_string()).dim(),
    );

    if let Some(id) = session_id {
        println!(
            "  {}  {} {}",
            style(r" (._.)").white(),
            style(id).dim(),
            style(format!("· {}", cwd_display)).dim(),
        );
    } else {
        println!(
            "  {}  {}",
            style(r" (._.)").white(),
            style(&cwd_display).dim(),
        );
    }
    let mode_note = if mode == GooseMode::Auto {
        style(" · tools run without asking · /mode to change")
            .dim()
            .to_string()
    } else {
        String::new()
    };
    println!(
        "  {}  {}{}",
        style(r"c(___)").white(),
        style("markov is ready").white(),
        mode_note,
    );
}

fn set_terminal_title() {
    if !std::io::stdout().is_terminal() {
        return;
    }
    let dir_name = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    // Sanitize: strip control characters (ESC, BEL, etc.) to prevent terminal escape injection
    let sanitized: String = dir_name.chars().filter(|c| !c.is_control()).collect();
    // OSC 0 sets the terminal window/tab title
    print!("\x1b]0;🪿 {}\x07", sanitized);
    let _ = std::io::stdout().flush();
}

/// The context gauge, as text. It is not printed: shown once per turn it would
/// leave a stale copy of itself glued to every past answer, so it belongs to
/// the input line, which the terminal clears on its own.
pub fn format_context_usage(total_tokens: usize, context_limit: usize) -> String {
    use console::style;

    if context_limit == 0 {
        return style("context usage unavailable (context limit is 0)")
            .dim()
            .to_string();
    }

    let percentage =
        (((total_tokens as f64 / context_limit as f64) * 100.0).round() as usize).min(100);

    let bar_width = 20;
    let filled = ((percentage as f64 / 100.0) * bar_width as f64).round() as usize;
    let empty = bar_width - filled.min(bar_width);

    let bar = format!("{}{}", "━".repeat(filled), "╌".repeat(empty));
    let colored_bar = if percentage < 50 {
        style(bar).green().dim()
    } else if percentage < 85 {
        style(bar).yellow()
    } else {
        style(bar).red()
    };

    fn format_tokens(n: usize) -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{:.0}k", n as f64 / 1_000.0)
        } else {
            n.to_string()
        }
    }

    format!(
        "{} {} {}",
        colored_bar,
        style(format!("{}%", percentage)).dim(),
        style(format!(
            "{}/{}",
            format_tokens(total_tokens),
            format_tokens(context_limit)
        ))
        .dim(),
    )
}

fn estimate_cost_usd(provider: &str, model: &str, usage: &Usage) -> Option<f64> {
    let canonical_model = maybe_get_canonical_model(provider, model)?;
    canonical_model.cost.estimate_cost(usage)
}

/// Display cost information, if price data is available.
pub fn display_cost_usage(provider: &str, model: &str, usage: &Usage) {
    if let Some(cost) = estimate_cost_usd(provider, model, usage) {
        use console::style;
        let input_tokens = usage.input_tokens.unwrap_or(0);
        let output_tokens = usage.output_tokens.unwrap_or(0);
        let cache_read = usage.cache_read_input_tokens.unwrap_or(0);
        let cache_write = usage.cache_write_input_tokens.unwrap_or(0);

        let cache_breakdown = match (cache_read, cache_write) {
            (0, 0) => String::new(),
            (read, 0) => format!(" ({} cache read)", read),
            (0, write) => format!(" ({} cache write)", write),
            (read, write) => format!(" ({} cache read, {} cache write)", read, write),
        };

        eprintln!(
            "Cost: {} USD ({} tokens: in {}{}, out {})",
            style(format!("${:.4}", cost)).cyan(),
            input_tokens + output_tokens,
            input_tokens,
            cache_breakdown,
            output_tokens
        );
    }
}

pub struct McpSpinners {
    bars: HashMap<String, ProgressBar>,
    log_spinner: Option<ProgressBar>,
    shell_output_lines: VecDeque<String>,
    multi_bar: MultiProgress,
}

impl McpSpinners {
    pub fn new() -> Self {
        McpSpinners {
            bars: HashMap::new(),
            log_spinner: None,
            shell_output_lines: VecDeque::new(),
            multi_bar: MultiProgress::new(),
        }
    }

    pub fn log(&mut self, message: &str) {
        let spinner = self.log_spinner.get_or_insert_with(|| {
            let bar = self.multi_bar.add(
                ProgressBar::new_spinner()
                    .with_style(
                        ProgressStyle::with_template("{spinner:.green} {msg}")
                            .unwrap()
                            .tick_chars("⠋⠙⠚⠛⠓⠒⠊⠉"),
                    )
                    .with_message(message.to_string()),
            );
            bar.enable_steady_tick(Duration::from_millis(100));
            bar
        });

        spinner.set_message(message.to_string());
    }

    pub fn log_shell_output(&mut self, lines: Vec<String>, max_lines: usize) {
        let message = update_recent_lines(&mut self.shell_output_lines, lines, max_lines);
        if !message.is_empty() {
            self.log(&message);
        }
    }

    pub fn update(&mut self, token: &str, value: f64, total: Option<f64>, message: Option<&str>) {
        let bar = self.bars.entry(token.to_string()).or_insert_with(|| {
            if let Some(total) = total {
                self.multi_bar.add(
                    ProgressBar::new((total * 100_f64) as u64).with_style(
                        ProgressStyle::with_template("[{elapsed}] {bar:40} {pos:>3}/{len:3} {msg}")
                            .unwrap(),
                    ),
                )
            } else {
                self.multi_bar.add(ProgressBar::new_spinner())
            }
        });
        bar.set_position((value * 100_f64) as u64);
        if let Some(msg) = message {
            bar.set_message(msg.to_string());
        }
    }

    pub fn hide(&mut self) -> Result<(), Error> {
        self.bars.iter_mut().for_each(|(_, bar)| {
            bar.disable_steady_tick();
        });
        if let Some(spinner) = self.log_spinner.as_mut() {
            spinner.disable_steady_tick();
        }
        self.shell_output_lines.clear();
        self.multi_bar.clear()
    }
}

fn update_recent_lines(
    recent_lines: &mut VecDeque<String>,
    lines: impl IntoIterator<Item = String>,
    max_lines: usize,
) -> String {
    recent_lines.extend(lines);
    while recent_lines.len() > max_lines {
        recent_lines.pop_front();
    }
    recent_lines
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n  ")
}

#[cfg(test)]
mod reply_layout_tests {
    use super::*;

    #[test]
    fn prose_is_wrapped_to_the_given_width() {
        let wrapped = wrap_markdown_source("one two three four five six", 9);
        assert_eq!(wrapped, "one two\nthree\nfour five\nsix");
    }

    #[test]
    fn a_fenced_block_is_left_as_written() {
        let source = "```\nlet the line of code run past the width\n```\n";
        assert_eq!(wrap_markdown_source(source, 10), source);
    }

    #[test]
    fn a_table_row_is_left_as_written() {
        let source = "| a long header | another long header |\n";
        assert_eq!(wrap_markdown_source(source, 10), source);
    }

    #[test]
    fn a_wrap_does_not_start_a_line_with_a_bullet() {
        // without the guard the break would fall as "alpha beta" / "- gamma"
        let wrapped = wrap_markdown_source("alpha beta - gamma", 11);
        assert_eq!(wrapped, "alpha\nbeta -\ngamma");
    }

    #[test]
    fn a_wrap_does_not_start_a_line_with_a_heading() {
        let wrapped = wrap_markdown_source("alpha beta ## gamma", 11);
        assert_eq!(wrapped, "alpha\nbeta ##\ngamma");
    }

    #[test]
    fn the_indent_of_a_wrapped_line_is_kept() {
        let wrapped = wrap_markdown_source("  one two three", 7);
        assert_eq!(wrapped, "  one\n  two\n  three");
    }

    #[test]
    fn the_marker_goes_on_the_first_line_and_the_indent_on_the_rest() {
        let block = indent_block("first\nsecond\n", Some("●"));
        assert_eq!(block, "\n● first\n  second\n");
    }

    #[test]
    fn a_blank_line_inside_a_block_stays_blank() {
        let block = indent_block("first\n\nsecond\n", Some("●"));
        assert_eq!(block, "\n● first\n\n  second\n");
    }

    #[test]
    fn only_a_marked_block_opens_with_an_empty_line() {
        assert!(indent_block("first\n", Some("●")).starts_with('\n'));
        assert!(!indent_block("first\n", None).starts_with('\n'));
    }

    #[test]
    fn a_block_without_a_marker_is_only_moved_in() {
        assert_eq!(indent_block("only\n", None), "  only\n");
    }

    #[test]
    fn the_context_gauge_is_returned_rather_than_printed() {
        let gauge = format_context_usage(7_000, 210_000);
        assert!(gauge.contains("3%"), "{gauge}");
        assert!(gauge.contains("7k/210k"), "{gauge}");
        assert!(!gauge.ends_with('\n'), "{gauge}");
    }
}

#[cfg(test)]
mod subagent_stream_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_first_call_of_a_subagent_spells_out_its_session_id() {
        let mut run = SubagentRun::default();
        assert_eq!(run.name("20260804_14", None), "[1] 20260804_14");
        assert_eq!(run.name("20260804_14", None), "[1]");
    }

    #[test]
    fn subagents_are_numbered_in_the_order_they_first_speak() {
        let mut run = SubagentRun::default();
        run.name("20260804_14", None);
        assert_eq!(run.name("20260804_16", None), "[2] 20260804_16");
        assert_eq!(run.name("20260804_14", None), "[1]");
    }

    #[test]
    fn a_task_that_says_what_it_is_is_named_by_it() {
        let mut run = SubagentRun::default();
        let name = run.name("20260804_14", Some("разобрать цикл reply"));
        assert_eq!(name, "[разобрать цикл reply] 20260804_14");
    }

    #[test]
    fn a_label_longer_than_the_budget_is_cut() {
        let mut run = SubagentRun::default();
        let long = "a".repeat(SUBAGENT_LABEL_BUDGET + 10);
        let name = run.name("20260804_14", Some(&long));
        let cut = safe_truncate(&long, SUBAGENT_LABEL_BUDGET);
        assert_eq!(name, format!("[{cut}] 20260804_14"));
    }

    #[test]
    fn an_empty_label_falls_back_to_the_number() {
        let mut run = SubagentRun::default();
        assert_eq!(run.name("20260804_14", Some("  ")), "[1] 20260804_14");
    }

    #[test]
    fn only_the_first_call_of_a_run_opens_a_block() {
        assert!(open_tool_run());
        assert!(!open_tool_run());

        end_tool_run();
        assert!(open_tool_run());
    }

    #[test]
    fn the_parameters_of_a_call_are_folded_onto_one_line() {
        // the order is the one the model wrote them in, which puts what the
        // call is about first
        let args = json!({"path": "src/main.rs", "limit": 20});
        let summary = summarize_params(args.as_object(), 80, false);
        assert_eq!(summary.as_deref(), Some("path src/main.rs  limit 20"));
    }

    #[test]
    fn a_parameter_of_several_lines_stays_on_one() {
        let args = json!({"content": "первая строка\n\nвторая строка"});
        let summary = summarize_params(args.as_object(), 80, false);
        assert_eq!(
            summary.as_deref(),
            Some("content первая строка вторая строка")
        );
    }

    #[test]
    fn a_summary_is_cut_to_the_room_it_has() {
        let args = json!({"command": "cargo tree -p goose --depth 1"});
        let summary = summarize_params(args.as_object(), 12, false).expect("expected a summary");
        assert_eq!(summary.chars().count(), 12);
    }

    #[test]
    fn a_call_without_parameters_has_nothing_to_show() {
        assert_eq!(summarize_params(None, 80, false), None);
        assert_eq!(summarize_params(json!({}).as_object(), 80, false), None);
        assert_eq!(
            summarize_params(json!({"source": null}).as_object(), 80, false),
            None
        );
    }

    #[test]
    fn a_long_path_is_shortened_before_it_is_measured() {
        let home = etcetera::home_dir().expect("expected a home dir");
        let path = home
            .join("dev/markov/crates/goose-cli/src/session/output.rs")
            .display()
            .to_string();
        let args = json!({ "path": path });
        let summary = summarize_params(args.as_object(), 200, false).expect("expected a summary");
        assert!(summary.ends_with("session/output.rs"), "{summary}");
        assert!(summary.chars().count() < path.chars().count(), "{summary}");
    }
}

#[cfg(test)]
mod tool_output_tests {
    use super::*;

    #[test]
    fn the_way_to_unfold_output_is_told_once() {
        assert_eq!(
            collapsed_output_note(312),
            "312 lines · /r shows full output"
        );
        assert_eq!(collapsed_output_note(7), "7 lines");
    }

    #[test]
    fn a_name_that_is_a_title_keeps_its_dunders() {
        let parts = display_parts("ls src/__pycache__", true);
        assert_eq!(parts.extension_name, None);
        assert_eq!(parts.tool_name, "ls src/__pycache__");

        let parts = display_parts("developer__shell", false);
        assert_eq!(parts.extension_name, Some("developer"));
        assert_eq!(parts.tool_name, "shell");
    }

    #[test]
    fn the_report_of_a_delegate_is_the_answer_of_the_turn() {
        assert!(answers_the_turn("load"));
        assert!(answers_the_turn("summon__load"));
        assert!(!answers_the_turn("shell"));
        assert!(!answers_the_turn("developer__download_file"));
    }

    #[test]
    fn the_calls_whose_result_is_the_answer_are_remembered_by_id() {
        remember_full_result("call_1");
        assert!(result_is_the_answer("call_1"));
        assert!(!result_is_the_answer("call_2"));

        for i in 0..FULL_RESULT_MEMORY {
            remember_full_result(&format!("later_{i}"));
        }
        assert!(!result_is_the_answer("call_1"));
    }
}

#[cfg(test)]
mod tool_parameter_tests {
    use super::*;

    #[test]
    fn a_value_of_several_lines_is_folded_when_previewed() {
        let folded = fit_value("первая\nвторая", Some(80), 4, false);
        assert_eq!(folded, "первая вторая");
    }

    #[test]
    fn a_value_shown_in_full_keeps_its_lines_behind_the_indent() {
        let shown = fit_value("первая\nвторая", Some(80), 4, true);
        assert_eq!(shown, "первая\n    вторая");
    }

    #[test]
    fn a_value_of_one_line_keeps_the_spacing_it_was_written_with() {
        assert_eq!(fit_value("rg  -n  foo", Some(80), 4, false), "rg  -n  foo");
    }

    #[test]
    fn a_value_is_cut_by_columns_and_not_by_bytes() {
        // eight letters of two bytes each fit a width of eight
        let cut = fit_value("привет!!", Some(8), 0, false);
        assert_eq!(cut, "привет!!");
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::*;

    #[test]
    fn words_are_kept_whole_when_wrapping() {
        assert_eq!(
            wrap_words("one two three four", 9),
            vec!["one two", "three", "four"]
        );
    }

    #[test]
    fn a_word_longer_than_the_width_gets_its_own_line() {
        assert_eq!(
            wrap_words("a loooooooooong tail", 4),
            vec!["a", "loooooooooong", "tail"]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_subagent_tool_call_names() {
        assert_eq!(
            format_subagent_tool_call_message("subagent_42", "read"),
            "[subagent:42] read"
        );
        assert_eq!(
            format_subagent_tool_call_message("subagent_42", "developer__shell"),
            "[subagent:42] shell | developer"
        );
        assert_eq!(
            format_subagent_tool_call_message("subagent_42", "code_execution__execute_typescript"),
            "[subagent:42] execute_typescript | Code Mode"
        );
        assert_eq!(
            format_subagent_tool_call_message("subagent_42", "calendar__events__list"),
            "[subagent:42] events__list | calendar"
        );
    }

    #[test]
    fn test_short_paths_unchanged() {
        assert_eq!(shorten_path("/usr/bin", false), "/usr/bin");
        assert_eq!(shorten_path("/a/b/c", false), "/a/b/c");
        assert_eq!(shorten_path("file.txt", false), "file.txt");
    }

    #[test]
    fn test_debug_mode_returns_full_path() {
        assert_eq!(
            shorten_path("/very/long/path/that/would/normally/be/shortened", true),
            "/very/long/path/that/would/normally/be/shortened"
        );
    }

    #[test]
    fn test_home_directory_conversion() {
        let _env = env_lock::lock_env([("HOME", Some("/Users/testuser"))]);

        assert_eq!(
            shorten_path("/Users/testuser/documents/file.txt", false),
            "~/documents/file.txt"
        );

        // A path that starts similarly to home but isn't in home
        assert_eq!(
            shorten_path("/Users/testuser2/documents/file.txt", false),
            "/Users/testuser2/documents/file.txt"
        );
    }

    #[test]
    fn test_toggle_full_tool_output() {
        let initial = get_show_full_tool_output();

        let after_first_toggle = toggle_full_tool_output();
        assert_eq!(after_first_toggle, !initial);
        assert_eq!(get_show_full_tool_output(), after_first_toggle);

        let after_second_toggle = toggle_full_tool_output();
        assert_eq!(after_second_toggle, initial);
        assert_eq!(get_show_full_tool_output(), initial);
    }

    #[test]
    fn test_long_path_shortening() {
        assert_eq!(
            shorten_path(
                "/vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv/long/path/with/many/components/file.txt",
                false
            ),
            "/v/l/p/w/m/components/file.txt"
        );
    }

    #[test]
    fn test_get_credits_top_up_url_from_credits_notification() {
        let message = Message::assistant().with_system_notification_with_data(
            SystemNotificationType::CreditsExhausted,
            "Insufficient credits",
            json!({"top_up_url": "https://router.tetrate.ai/billing"}),
        );
        assert_eq!(
            get_credits_top_up_url(&message).as_deref(),
            Some("https://router.tetrate.ai/billing")
        );
    }

    #[test]
    fn content_without_priority_is_visible_by_default() {
        assert!(is_visible_to_user(None, DEFAULT_MIN_PRIORITY, false));
    }

    #[test]
    fn content_without_priority_respects_a_raised_threshold() {
        assert!(!is_visible_to_user(None, 0.5, false));
    }

    #[test]
    fn annotated_priority_at_the_threshold_is_visible() {
        let annotations = Annotations::default().with_priority(0.0);
        assert!(is_visible_to_user(
            Some(&annotations),
            DEFAULT_MIN_PRIORITY,
            false
        ));
    }

    #[test]
    fn assistant_only_content_is_hidden() {
        let annotations = Annotations::default().with_audience(vec![Role::Assistant]);
        assert!(!is_visible_to_user(
            Some(&annotations),
            DEFAULT_MIN_PRIORITY,
            false
        ));
    }

    #[test]
    fn errors_ignore_the_priority_threshold() {
        let annotations = Annotations::default().with_priority(0.1);
        assert!(is_visible_to_user(Some(&annotations), 0.5, true));
    }

    #[test]
    fn errors_still_respect_the_audience() {
        let annotations = Annotations::default().with_audience(vec![Role::Assistant]);
        assert!(!is_visible_to_user(
            Some(&annotations),
            DEFAULT_MIN_PRIORITY,
            true
        ));
    }

    #[test]
    fn test_get_credits_top_up_url_ignores_non_credits_notification() {
        let message = Message::assistant().with_system_notification_with_data(
            SystemNotificationType::InlineMessage,
            "hello",
            json!({"top_up_url": "https://router.tetrate.ai/billing"}),
        );
        assert_eq!(get_credits_top_up_url(&message), None);
    }
}
