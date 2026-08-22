//! Pieces of the reply loop that are ours, kept out of the upstream module so
//! upstream's edits around them merge on their own.

use std::time::Duration;
use tokio::signal::ctrl_c;

/// How long a closing session waits for the name the model is still writing.
pub const SESSION_NAME_GRACE: Duration = Duration::from_secs(3);

/// Appends the models the cache does not have yet. What is already there came
/// from the provider metadata in a deliberate order, so it keeps its place and
/// only the newcomers are sorted.
pub fn merge_fetched_models(known: &mut Vec<String>, fetched: Vec<String>) {
    let mut fresh: Vec<String> = fetched
        .into_iter()
        .filter(|model| !known.contains(model))
        .collect();
    fresh.sort();
    fresh.dedup();
    known.extend(fresh);
}

/// Runs a call until it finishes or the interrupt future fires first. The
/// planner has no reply loop watching for Ctrl-C the way a normal turn does,
/// so its calls to the model are raced against the signal here.
pub async fn until_interrupted<T>(
    call: impl std::future::Future<Output = T>,
    interrupt: impl std::future::Future<Output = ()>,
) -> Option<T> {
    tokio::select! {
        result = call => Some(result),
        _ = interrupt => None,
    }
}

pub async fn wait_for_ctrl_c() {
    let _ = ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetched_models_are_appended_after_the_known_ones() {
        let mut known = vec!["curated-2".to_string(), "curated-1".to_string()];
        merge_fetched_models(&mut known, vec!["b".to_string(), "a".to_string()]);
        assert_eq!(known, ["curated-2", "curated-1", "a", "b"]);
    }

    #[test]
    fn a_model_already_known_is_not_repeated() {
        let mut known = vec!["shared".to_string()];
        merge_fetched_models(&mut known, vec!["shared".to_string(), "new".to_string()]);
        assert_eq!(known, ["shared", "new"]);
    }

    #[test]
    fn a_gateway_repeating_itself_yields_one_entry() {
        let mut known = Vec::new();
        merge_fetched_models(&mut known, vec!["one".to_string(), "one".to_string()]);
        assert_eq!(known, ["one"]);
    }

    #[tokio::test]
    async fn a_call_that_finishes_is_returned_whole() {
        let result = until_interrupted(std::future::ready(7), std::future::pending()).await;
        assert_eq!(result, Some(7));
    }

    #[tokio::test]
    async fn an_interrupt_beats_a_call_that_hangs() {
        let result = until_interrupted(std::future::pending::<i32>(), std::future::ready(())).await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn a_call_that_fails_reports_its_error_rather_than_an_interrupt() {
        let call = std::future::ready(Err::<(), _>(anyhow::anyhow!("boom")));
        let result = until_interrupted(call, std::future::pending()).await;
        assert!(result.is_some_and(|inner| inner.is_err()));
    }
}
