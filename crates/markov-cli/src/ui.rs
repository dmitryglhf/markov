//! Pieces the interactive dialogs share. Everything here earned its place by
//! having a second caller; whatever only one form needs stays in that form.

use anyhow::Result;
use cliclack::{MultiSelect, Select};
use console::{measure_text_width, Term};
use std::fmt::Display;
use std::io::IsTerminal;

/// A list longer than this scrolls instead of filling the screen. The filter is
/// switched on at the same point: below it every item is already visible, and
/// its input line reads as a stray cursor above the list.
///
/// Raised from ten once the extension list arrived: at twelve rows it was
/// scrolling and offering a filter for a dozen short names, which is more
/// apparatus than the list is worth.
pub const VISIBLE_ROWS: usize = 15;

pub fn terminal_width() -> Option<usize> {
    Term::stdout()
        .size_checked()
        .map(|(_height, width)| width as usize)
}

pub fn truncate_to_display_width(text: &str, max_width: usize) -> String {
    if measure_text_width(text) <= max_width {
        return text.to_string();
    }

    if max_width <= 3 {
        return ".".repeat(max_width);
    }

    let mut output = String::new();
    let suffix_width = measure_text_width("...");

    for ch in text.chars() {
        output.push(ch);
        if measure_text_width(&output) + suffix_width > max_width {
            output.pop();
            break;
        }
    }

    output.push_str("...");
    output
}

pub fn pad_to_display_width(text: &str, width: usize) -> String {
    let padding = width.saturating_sub(measure_text_width(text));
    format!("{}{}", text, " ".repeat(padding))
}

/// cliclack reports Ctrl-C as an interrupted read. Inside a session that has to
/// close the form, not tear down the REPL.
pub fn cancellable<T>(result: std::io::Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// The same interrupted read, seen through a helper that already wrapped it in
/// `anyhow`. A form that shares its steps with `markov configure` gets errors in
/// that shape and still has to tell Ctrl-C apart from a real failure.
pub fn is_cancel(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::Interrupted)
}

pub fn require_terminal(command: &str) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("`{command}` needs a terminal. Use `markov configure` from scripts.");
    }
    Ok(())
}

pub fn select<T, L, H>(prompt: impl Display, items: &[(T, L, H)]) -> Select<T>
where
    T: Clone + Eq,
    L: Display,
    H: Display,
{
    let list = cliclack::select(prompt).items(items).max_rows(VISIBLE_ROWS);
    match items.len() > VISIBLE_ROWS {
        true => list.filter_mode(),
        false => list,
    }
}

pub fn multiselect<T, L, H>(prompt: impl Display, items: &[(T, L, H)]) -> MultiSelect<T>
where
    T: Clone + Eq,
    L: Display,
    H: Display,
{
    let list = cliclack::multiselect(prompt)
        .items(items)
        .max_rows(VISIBLE_ROWS);
    match items.len() > VISIBLE_ROWS {
        true => list.filter_mode(),
        false => list,
    }
}
