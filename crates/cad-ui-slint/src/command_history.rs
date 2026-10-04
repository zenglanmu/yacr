//! Bounded command recall for UI requirements 10 and 15.
//!
//! This model stores submitted text, not execution results or output messages.
//! The caller decides when to record a command and how to display rejection.
//! Text is never normalized or truncated: paths and arguments retain their case.
//! Budgets cover retained command UTF-8 bytes, excluding the live editor draft.

use std::collections::VecDeque;

const MAX_ENTRIES: usize = 50;
const MAX_ENTRY_BYTES: usize = 4096;
const MAX_TOTAL_BYTES: usize = 64 * 1024;

/// Recording never silently substitutes a shortened executable command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecordOutcome {
    Stored,
    Empty,
    Duplicate,
    Oversized,
}

/// Oldest-first bounded recall, independent of Slint and command dispatch.
#[derive(Debug, Default)]
pub(crate) struct CommandHistory {
    commands: VecDeque<String>,
    total_bytes: usize,
    cursor: Option<usize>,
    draft: String,
}

impl CommandHistory {
    /// Reset navigation on every submission, including rejected submissions.
    /// Whitespace-only input is ignored; otherwise preserve the exact input.
    /// Only exact consecutive duplicates are suppressed.
    pub(crate) fn record(&mut self, command: &str) -> RecordOutcome {
        self.reset_navigation();
        if command.len() > MAX_ENTRY_BYTES {
            return RecordOutcome::Oversized;
        }
        if command.trim().is_empty() {
            return RecordOutcome::Empty;
        }
        if self.commands.back().is_some_and(|last| last == command) {
            return RecordOutcome::Duplicate;
        }
        while self.commands.len() >= MAX_ENTRIES
            || self.total_bytes + command.len() > MAX_TOTAL_BYTES
        {
            if let Some(oldest) = self.commands.pop_front() {
                self.total_bytes -= oldest.len();
            }
        }
        self.commands.push_back(command.to_owned());
        self.total_bytes += command.len();
        RecordOutcome::Stored
    }

    /// Recall older (-1) or newer (+1) text without wrapping at either end.
    /// The first older request saves the draft; later requests cannot replace it.
    /// Returning past the newest command restores the draft and ends navigation.
    /// Unknown directions leave both the supplied text and navigation unchanged.
    pub(crate) fn recall(&mut self, direction: i32, draft: &str) -> String {
        match direction {
            -1 if !self.commands.is_empty() => {
                let index = match self.cursor {
                    Some(index) => index.saturating_sub(1),
                    None => {
                        self.draft = draft.to_owned();
                        self.commands.len() - 1
                    }
                };
                self.cursor = Some(index);
                self.commands[index].clone()
            }
            1 => match self.cursor {
                Some(index) if index + 1 < self.commands.len() => {
                    self.cursor = Some(index + 1);
                    self.commands[index + 1].clone()
                }
                Some(_) => {
                    self.cursor = None;
                    std::mem::take(&mut self.draft)
                }
                None => draft.to_owned(),
            },
            _ => draft.to_owned(),
        }
    }

    /// Clear retained commands, byte accounting, and any saved navigation draft.
    pub(crate) fn clear(&mut self) {
        self.commands.clear();
        self.total_bytes = 0;
        self.reset_navigation();
    }

    /// Borrow retained command text in chronological (oldest-to-newest) order.
    pub(crate) fn entries(&self) -> impl Iterator<Item = &str> {
        self.commands.iter().map(String::as_str)
    }

    fn reset_navigation(&mut self) {
        self.cursor = None;
        self.draft = String::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn populated() -> CommandHistory {
        let mut history = CommandHistory::default();
        history.record("first");
        history.record("second");
        history.record("third");
        history
    }

    #[test]
    fn default_and_empty_navigation_preserve_input() {
        let mut history = CommandHistory::default();
        assert_eq!(history.entries().count(), 0);
        assert_eq!(history.total_bytes, 0);
        for direction in [-1, 1, 0, i32::MIN, i32::MAX] {
            assert_eq!(
                history.recall(direction, "Unfinished 草稿"),
                "Unfinished 草稿"
            );
            assert_eq!(history.cursor, None);
            assert!(history.draft.is_empty());
        }
    }

    #[test]
    fn whitespace_only_commands_are_not_stored() {
        let mut history = CommandHistory::default();
        for command in ["", " ", "\t\r\n", "\u{2003}\u{3000}"] {
            assert_eq!(history.record(command), RecordOutcome::Empty);
        }
        assert_eq!(history.entries().count(), 0);
        assert_eq!(history.total_bytes, 0);
    }

    #[test]
    fn original_case_whitespace_and_unicode_are_preserved() {
        let mut history = CommandHistory::default();
        let command = "  open /Drawings/混合Case🙂.dwg\t";
        assert_eq!(history.record(command), RecordOutcome::Stored);
        assert_eq!(history.entries().collect::<Vec<_>>(), vec![command]);
        assert_eq!(history.recall(-1, "draft"), command);
        assert_eq!(history.total_bytes, command.len());
    }

    #[test]
    fn duplicate_suppression_is_exact_and_consecutive() {
        let mut history = CommandHistory::default();
        assert_eq!(history.record("OPEN"), RecordOutcome::Stored);
        assert_eq!(history.record("OPEN"), RecordOutcome::Duplicate);
        assert_eq!(history.record("open"), RecordOutcome::Stored);
        assert_eq!(history.record(" open"), RecordOutcome::Stored);
        assert_eq!(history.record("OPEN"), RecordOutcome::Stored);
        assert_eq!(
            history.entries().collect::<Vec<_>>(),
            vec!["OPEN", "open", " open", "OPEN"]
        );
        assert_eq!(history.total_bytes, 17);
    }

    #[test]
    fn navigation_clamps_at_oldest_and_restores_original_draft() {
        let mut history = populated();
        assert_eq!(history.recall(-1, "my 草稿"), "third");
        assert_eq!(history.recall(-1, "third edited"), "second");
        assert_eq!(history.recall(-1, "second edited"), "first");
        assert_eq!(history.recall(-1, "first edited"), "first");
        assert_eq!(history.recall(1, "ignored"), "second");
        assert_eq!(history.recall(1, "ignored"), "third");
        assert_eq!(history.recall(1, "ignored"), "my 草稿");
        assert_eq!(history.cursor, None);
        assert!(history.draft.is_empty());
        assert_eq!(history.recall(1, "new draft"), "new draft");
    }

    #[test]
    fn reversing_navigation_does_not_replace_saved_draft() {
        let mut history = populated();
        history.recall(-1, "original");
        history.recall(-1, "edited");
        history.recall(1, "edited again");
        history.recall(-1, "another edit");
        history.recall(1, "ignored");
        assert_eq!(history.recall(1, "ignored"), "original");
    }

    #[test]
    fn a_new_navigation_session_captures_a_new_draft() {
        let mut history = populated();
        assert_eq!(history.recall(1, "before navigation"), "before navigation");
        assert_eq!(history.recall(-1, "first draft"), "third");
        assert_eq!(history.recall(1, "third"), "first draft");
        assert_eq!(history.recall(-1, "second draft"), "third");
        assert_eq!(history.recall(1, "third"), "second draft");
    }

    #[test]
    fn unknown_directions_do_not_mutate_navigation() {
        let mut history = populated();
        history.recall(-1, "saved");
        for direction in [0, -2, 2, i32::MIN, i32::MAX] {
            assert_eq!(history.recall(direction, "current"), "current");
            assert_eq!(history.cursor, Some(2));
            assert_eq!(history.draft, "saved");
        }
        assert_eq!(history.recall(1, "ignored"), "saved");
    }

    #[test]
    fn empty_draft_is_restored_exactly() {
        let mut history = populated();
        history.recall(-1, "");
        assert_eq!(history.recall(1, "edited command"), "");
    }

    #[test]
    fn every_record_outcome_resets_navigation() {
        for (command, outcome) in [
            ("new command".to_owned(), RecordOutcome::Stored),
            ("third".to_owned(), RecordOutcome::Duplicate),
            ("\u{3000}".to_owned(), RecordOutcome::Empty),
            ("x".repeat(MAX_ENTRY_BYTES + 1), RecordOutcome::Oversized),
        ] {
            let mut history = populated();
            history.recall(-1, "stale draft");
            assert_eq!(history.record(&command), outcome);
            assert_eq!(history.cursor, None);
            assert!(history.draft.is_empty());
            assert_eq!(history.recall(1, "fresh draft"), "fresh draft");
            history.recall(-1, "fresh draft");
            assert_eq!(history.recall(1, "ignored"), "fresh draft");
        }
    }

    #[test]
    fn count_budget_evicts_oldest_entries_only() {
        let mut history = CommandHistory::default();
        for index in 0..MAX_ENTRIES + 7 {
            assert_eq!(
                history.record(&format!("command {index}")),
                RecordOutcome::Stored
            );
        }
        let expected: Vec<_> = (7..MAX_ENTRIES + 7)
            .map(|index| format!("command {index}"))
            .collect();
        assert_eq!(
            history.entries().collect::<Vec<_>>(),
            expected.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert_eq!(
            history.total_bytes,
            expected.iter().map(String::len).sum::<usize>()
        );
    }

    #[test]
    fn exact_entry_byte_limit_accepts_unicode_without_modification() {
        let mut history = CommandHistory::default();
        let command = "🙂".repeat(MAX_ENTRY_BYTES / 4);
        assert_eq!(command.len(), MAX_ENTRY_BYTES);
        assert_eq!(history.record(&command), RecordOutcome::Stored);
        assert_eq!(history.recall(-1, ""), command);
        assert_eq!(history.total_bytes, MAX_ENTRY_BYTES);
    }

    #[test]
    fn oversized_unicode_is_rejected_without_truncation_or_eviction() {
        let mut history = populated();
        let before: Vec<_> = history.entries().map(str::to_owned).collect();
        let before_bytes = history.total_bytes;
        let command = format!("{}é", "x".repeat(MAX_ENTRY_BYTES - 1));
        assert_eq!(command.len(), MAX_ENTRY_BYTES + 1);
        assert_eq!(history.record(&command), RecordOutcome::Oversized);
        assert_eq!(
            history.entries().collect::<Vec<_>>(),
            before.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert_eq!(history.total_bytes, before_bytes);
    }

    #[test]
    fn total_byte_budget_evicts_oldest_before_count_limit() {
        let mut history = CommandHistory::default();
        let retained = MAX_TOTAL_BYTES / MAX_ENTRY_BYTES;
        for index in 0..retained {
            let command = format!("{index:04}{}", "x".repeat(MAX_ENTRY_BYTES - 4));
            assert_eq!(history.record(&command), RecordOutcome::Stored);
        }
        assert_eq!(history.total_bytes, MAX_TOTAL_BYTES);
        assert_eq!(history.entries().count(), retained);
        assert_eq!(history.record("next"), RecordOutcome::Stored);
        assert_eq!(history.entries().count(), retained);
        assert!(history.entries().next().unwrap().starts_with("0001"));
        assert_eq!(history.entries().last(), Some("next"));
        assert_eq!(history.total_bytes, MAX_TOTAL_BYTES - MAX_ENTRY_BYTES + 4);
    }

    #[test]
    fn byte_budget_can_require_multiple_evictions() {
        let mut history = CommandHistory::default();
        for index in 0..32 {
            let command = format!("{index:04}{}", "x".repeat(2044));
            history.record(&command);
        }
        assert_eq!(history.total_bytes, MAX_TOTAL_BYTES);
        assert_eq!(
            history.record(&"y".repeat(MAX_ENTRY_BYTES)),
            RecordOutcome::Stored
        );
        assert_eq!(history.entries().count(), 31);
        assert!(history.entries().next().unwrap().starts_with("0002"));
        assert_eq!(history.total_bytes, MAX_TOTAL_BYTES);
    }

    #[test]
    fn duplicate_does_not_evict_or_change_byte_accounting_at_capacity() {
        let mut history = CommandHistory::default();
        for index in 0..MAX_ENTRIES {
            history.record(&format!("command {index}"));
        }
        let before_bytes = history.total_bytes;
        assert_eq!(history.record("command 49"), RecordOutcome::Duplicate);
        assert_eq!(history.entries().count(), MAX_ENTRIES);
        assert_eq!(history.entries().next(), Some("command 0"));
        assert_eq!(history.total_bytes, before_bytes);
    }

    #[test]
    fn clear_resets_navigation_and_allows_reuse() {
        let mut history = populated();
        history.recall(-1, "stale");
        history.clear();
        history.clear();
        assert_eq!(history.entries().count(), 0);
        assert_eq!(history.total_bytes, 0);
        assert_eq!(history.cursor, None);
        assert!(history.draft.is_empty());
        assert_eq!(history.recall(-1, "fresh"), "fresh");
        assert_eq!(history.recall(1, "fresh"), "fresh");
        assert_eq!(history.record("third"), RecordOutcome::Stored);
        assert_eq!(history.recall(-1, "fresh"), "third");
        assert_eq!(history.recall(1, "ignored"), "fresh");
        assert_eq!(history.total_bytes, 5);
    }

    #[test]
    fn mixed_records_keep_exact_accounting_within_both_budgets() {
        let mut history = CommandHistory::default();
        for index in 0..200 {
            let command = format!("{index}:{}", "界".repeat(index % 1300));
            assert_eq!(history.record(&command), RecordOutcome::Stored);
            assert_eq!(history.record(&command), RecordOutcome::Duplicate);
            assert_eq!(history.record("\t"), RecordOutcome::Empty);
            assert!(history.commands.len() <= MAX_ENTRIES);
            assert!(history.total_bytes <= MAX_TOTAL_BYTES);
            assert!(history
                .entries()
                .all(|entry| entry.len() <= MAX_ENTRY_BYTES));
            assert_eq!(
                history.total_bytes,
                history.entries().map(str::len).sum::<usize>()
            );
        }
    }
}
