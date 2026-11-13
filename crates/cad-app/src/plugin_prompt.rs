//! The plugin `Editor` prompt state: one asynchronous prompt, answered by the
//! host's ordinary input funnel.
//!
//! The WebView cannot see native clicks, so a prompt lives in the **session**
//! (here, on the controller the owner thread owns) and is resolved by the same
//! interactions a user already produces: a point capture, a click pick, typed
//! text, Enter and Esc. A resolved prompt is stored as an [`ApiPromptResult`] and
//! travels back to the plugin through the existing submit/poll lane — never as a
//! return value of the submit call.
//!
//! # One prompt at a time
//!
//! [`HostController::begin_api_prompt`] refuses while another prompt is pending
//! (`api.prompt.busy`) and while the completed-answer queue is full
//! ([`API_PROMPT_RESULTS_FULL`]). A settled answer is **never** dropped: the
//! controller refuses new prompts instead, so the queue is bounded by
//! [`CAD_API_PROMPT_RESULT_CAP`] plus at most the one prompt that was already
//! active when it filled.
//!
//! # Re-prompt semantics
//!
//! Invalid input does not produce an answer: the prompt stays pending and
//! [`HostController::api_prompt_error`] carries the sentence explaining why. The
//! next valid input completes it and clears the sentence.
//!
//! Nothing here writes `cad-db`; a prompt only reads geometry/selection and
//! publishes a selection the same way `SELECTALL`/`Select` already do.

use crate::host::HostController;
use crate::{Command, CommandId, CommandPayload, EntityId};
use cad_domain::{CadError, CadResult, Point3};

/// Stable refusal of a second prompt while one is already pending.
pub const API_PROMPT_BUSY: &str = "api.prompt.busy";

/// Stable refusal of a prompt while the completed-answer queue is full.
pub const API_PROMPT_RESULTS_FULL: &str = "cad_api.prompt.results_full";

/// Bound on completed answers the controller keeps until the submit lane takes
/// them (see the module docs for the exact bound).
pub const CAD_API_PROMPT_RESULT_CAP: usize = 32;

/// Which value a pending prompt asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiPromptKind {
    Point,
    Entity,
    Selection,
    String,
    Double,
    Integer,
    Keyword,
}

impl ApiPromptKind {
    /// The lowercase wire spelling of the kind (`pendingPrompt.kind`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::Entity => "entity",
            Self::Selection => "selection",
            Self::String => "string",
            Self::Double => "double",
            Self::Integer => "integer",
            Self::Keyword => "keyword",
        }
    }

    /// The prompt sentence used when the caller supplies none.
    fn default_message(self) -> &'static str {
        match self {
            Self::Point => "Specify point:",
            Self::Entity => "Select object:",
            Self::Selection => "Select objects:",
            Self::String => "Enter text:",
            Self::Double => "Enter a number:",
            Self::Integer => "Enter an integer:",
            Self::Keyword => "Enter an option:",
        }
    }
}

/// A prompt's accepted default (Enter with no input).
#[derive(Debug, Clone, PartialEq)]
pub enum ApiPromptDefault {
    /// A string default (`getString`).
    String(String),
    /// A number default (`getDouble`/`getInteger`).
    Number(f64),
}

/// One pending prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiPrompt {
    /// The submit request whose answer this prompt will complete.
    pub request_id: String,
    /// What the prompt asks for.
    pub kind: ApiPromptKind,
    /// The sentence shown to the user.
    pub message: String,
    /// Accepted keywords, in the caller's spelling.
    pub keywords: Vec<String>,
    /// What Enter accepts when nothing is typed.
    pub default: Option<ApiPromptDefault>,
    /// `getEntity`'s sentence for a click that hits nothing.
    pub reject_message: Option<String>,
}

impl ApiPrompt {
    /// A prompt of `kind` for `request_id`, with the host's default sentence when
    /// the caller supplied none.
    pub fn new(
        request_id: impl Into<String>,
        kind: ApiPromptKind,
        message: Option<String>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            kind,
            message: message
                .filter(|message| !message.trim().is_empty())
                .unwrap_or_else(|| kind.default_message().to_string()),
            keywords: Vec::new(),
            default: None,
            reject_message: None,
        }
    }
}

/// The settled answer of one prompt (mapped to JSON by the shared layer).
#[derive(Debug, Clone, PartialEq)]
pub enum ApiPromptReply {
    /// A captured point (world coordinates).
    Point(Point3),
    /// The picked entity.
    Entity(EntityId),
    /// The published selection (deduplicated entity ids).
    Selection(Vec<EntityId>),
    /// A typed string.
    String(String),
    /// A parsed number (`getDouble` keeps it as-is; `getInteger` is whole).
    Number(f64),
    /// A matched keyword, in the prompt's own spelling.
    Keyword(String),
    /// Enter with no value and no default.
    None,
    /// Esc (or an explicit cancel).
    Cancel,
}

/// One completed prompt, waiting for the submit lane to take it.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiPromptResult {
    /// The submit request this answer belongs to.
    pub request_id: String,
    /// The answer.
    pub reply: ApiPromptReply,
}

/// One input the host funnel offers to a pending prompt.
#[derive(Debug, Clone, PartialEq)]
pub enum ApiPromptInput {
    /// A captured point (already resolved to world coordinates).
    Point(Point3),
    /// The entity a click picked.
    Entity(EntityId),
    /// A click that hit nothing (`getEntity` re-prompts with its reject sentence).
    EntityMiss,
    /// A selection click was applied to the session selection.
    SelectionChanged,
    /// Text typed at the prompt.
    Text(String),
    /// Enter.
    Confirm,
    /// Esc.
    Cancel,
}

/// What the input funnel should do with an offered input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiPromptDisposition {
    /// No prompt was pending: run the ordinary tool/navigation path.
    NotPrompted,
    /// A pending prompt consumed the input (completed or re-prompted).
    Handled,
}

impl HostController {
    /// Start one prompt, refusing while another is pending or the answer queue is
    /// full.
    pub fn begin_api_prompt(&mut self, prompt: ApiPrompt) -> CadResult<()> {
        if self.api_prompt.is_some() {
            return Err(CadError::InvalidInput(API_PROMPT_BUSY.to_string()));
        }
        if self.api_prompt_results.len() >= CAD_API_PROMPT_RESULT_CAP {
            return Err(CadError::InvalidInput(API_PROMPT_RESULTS_FULL.to_string()));
        }
        self.api_prompt = Some(prompt);
        self.api_prompt_error = None;
        Ok(())
    }

    /// The pending prompt, when one exists.
    pub fn api_prompt(&self) -> Option<&ApiPrompt> {
        self.api_prompt.as_ref()
    }

    /// The last invalid-input sentence while a prompt stays pending.
    pub fn api_prompt_error(&self) -> Option<&str> {
        self.api_prompt_error.as_deref()
    }

    /// Whether a prompt currently owns the prompt-bearing inputs.
    pub fn api_prompt_pending(&self) -> bool {
        self.api_prompt.is_some()
    }

    /// Offer one input to the pending prompt.
    ///
    /// A completed answer is appended to [`Self::api_prompt_results`]; invalid
    /// input keeps the prompt and records [`Self::api_prompt_error`]. When no
    /// prompt is pending the input belongs to the ordinary path
    /// ([`ApiPromptDisposition::NotPrompted`]).
    pub fn resolve_api_prompt(&mut self, input: ApiPromptInput) -> ApiPromptDisposition {
        let Some(prompt) = self.api_prompt.clone() else {
            return ApiPromptDisposition::NotPrompted;
        };
        match input {
            ApiPromptInput::Cancel => self.finish_api_prompt(ApiPromptReply::Cancel),
            ApiPromptInput::Point(point) => match prompt.kind {
                ApiPromptKind::Point => self.finish_api_prompt(ApiPromptReply::Point(point)),
                // A click during a text/object prompt is consumed, never an error.
                _ => self.pending_without_error(),
            },
            ApiPromptInput::Entity(entity) => match prompt.kind {
                ApiPromptKind::Entity => self.finish_api_prompt(ApiPromptReply::Entity(entity)),
                _ => self.pending_without_error(),
            },
            ApiPromptInput::EntityMiss => match prompt.kind {
                ApiPromptKind::Entity => self.reprompt(
                    prompt
                        .reject_message
                        .as_deref()
                        .unwrap_or("invalid input: no object found there"),
                ),
                _ => self.pending_without_error(),
            },
            ApiPromptInput::SelectionChanged => {
                // The click already published the selection; keep prompting.
                if prompt.kind == ApiPromptKind::Selection {
                    self.pending_without_error()
                } else {
                    ApiPromptDisposition::Handled
                }
            }
            ApiPromptInput::Text(text) => match typed_reply(&prompt, &text) {
                Ok(reply) => self.finish_api_prompt(reply),
                Err(sentence) => self.reprompt(&sentence),
            },
            ApiPromptInput::Confirm => match prompt.kind {
                ApiPromptKind::Selection => {
                    let ids = self.selected_entity_ids();
                    self.finish_api_prompt(ApiPromptReply::Selection(ids))
                }
                ApiPromptKind::String
                | ApiPromptKind::Double
                | ApiPromptKind::Integer
                | ApiPromptKind::Keyword => {
                    let reply = match prompt.default {
                        Some(ApiPromptDefault::String(value)) => ApiPromptReply::String(value),
                        Some(ApiPromptDefault::Number(value)) => ApiPromptReply::Number(value),
                        None => ApiPromptReply::None,
                    };
                    self.finish_api_prompt(reply)
                }
                // Enter alone has no point/entity to report: the prompt stays
                // pending and nothing is invented.
                ApiPromptKind::Point | ApiPromptKind::Entity => self.pending_without_error(),
            },
        }
    }

    /// Cancel the pending prompt (idempotent: no prompt, no answer).
    pub fn cancel_api_prompt(&mut self) {
        if self.api_prompt.is_some() {
            self.finish_api_prompt(ApiPromptReply::Cancel);
        }
    }

    /// Take every completed answer, oldest first.
    pub fn take_api_prompt_results(&mut self) -> Vec<ApiPromptResult> {
        std::mem::take(&mut self.api_prompt_results)
    }

    /// The session selection's entity ids, in selection order, deduplicated.
    fn selected_entity_ids(&self) -> Vec<EntityId> {
        let mut ids: Vec<EntityId> = Vec::new();
        for reference in self.session.selection.refs() {
            if !ids.contains(&reference.entity) {
                ids.push(reference.entity);
            }
        }
        ids
    }

    /// Complete the pending prompt with `reply` (a no-op when none is pending).
    fn finish_api_prompt(&mut self, reply: ApiPromptReply) -> ApiPromptDisposition {
        let request_id = self.api_prompt.take().map(|prompt| prompt.request_id);
        self.api_prompt_error = None;
        if let Some(request_id) = request_id {
            self.api_prompt_results
                .push(ApiPromptResult { request_id, reply });
        }
        ApiPromptDisposition::Handled
    }

    fn reprompt(&mut self, sentence: &str) -> ApiPromptDisposition {
        self.api_prompt_error = Some(sentence.to_string());
        ApiPromptDisposition::Handled
    }

    fn pending_without_error(&mut self) -> ApiPromptDisposition {
        self.api_prompt_error = None;
        ApiPromptDisposition::Handled
    }

    /// Run the existing `SELECTALL` path and answer the ids it published.
    pub fn plugin_select_all(&mut self) -> CadResult<Vec<EntityId>> {
        self.execute(Command {
            schema_version: 1,
            id: CommandId::SelectAll,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: CommandPayload::None,
        })?;
        Ok(self.selected_entity_ids())
    }
}

/// Parse typed text against the pending prompt's kind and keywords.
fn typed_reply(prompt: &ApiPrompt, text: &str) -> Result<ApiPromptReply, String> {
    let trimmed = text.trim();
    if let Some(keyword) = match_keyword(&prompt.keywords, trimmed) {
        return Ok(ApiPromptReply::Keyword(keyword));
    }
    match prompt.kind {
        ApiPromptKind::Point => parse_point(trimmed)
            .map(ApiPromptReply::Point)
            .ok_or_else(|| "invalid input: enter a point as x,y or x,y,z".to_string()),
        ApiPromptKind::Entity => {
            Err("invalid input: click the object this prompt asks for".to_string())
        }
        ApiPromptKind::Selection => {
            Err("invalid input: click objects, then press Enter".to_string())
        }
        ApiPromptKind::String => {
            if text.is_empty() {
                Ok(empty_default_reply(prompt))
            } else {
                Ok(ApiPromptReply::String(text.to_string()))
            }
        }
        ApiPromptKind::Double => match trimmed.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(ApiPromptReply::Number(value)),
            _ => Err("invalid input: enter a number".to_string()),
        },
        ApiPromptKind::Integer => match trimmed.parse::<i64>() {
            // `f64` is exact only up to 2^53; anything larger cannot round-trip
            // through the JSON number the client reads as a safe integer.
            Ok(value) if value.unsigned_abs() <= (1_u64 << 53) => {
                Ok(ApiPromptReply::Number(value as f64))
            }
            _ => Err("invalid input: enter a whole number".to_string()),
        },
        ApiPromptKind::Keyword => Err(expected_keywords(&prompt.keywords)),
    }
}

/// Enter on an empty prompt: the default, or the honest `none`.
fn empty_default_reply(prompt: &ApiPrompt) -> ApiPromptReply {
    match &prompt.default {
        Some(ApiPromptDefault::String(value)) => ApiPromptReply::String(value.clone()),
        Some(ApiPromptDefault::Number(value)) => ApiPromptReply::Number(*value),
        None => ApiPromptReply::None,
    }
}

/// Match `text` against the keywords case-insensitively, returning the caller's
/// own spelling.
fn match_keyword(keywords: &[String], text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    keywords
        .iter()
        .find(|keyword| keyword.eq_ignore_ascii_case(text))
        .cloned()
}

/// The sentence for text that is not one of the prompt's keywords.
fn expected_keywords(keywords: &[String]) -> String {
    if keywords.is_empty() {
        "invalid input: no keyword was offered".to_string()
    } else {
        format!("invalid input: expected one of {}", keywords.join(", "))
    }
}

/// Parse `x,y` or `x,y,z` (finite numbers).
fn parse_point(text: &str) -> Option<Point3> {
    let mut parts = text.split(',');
    let x = parts.next()?.trim().parse::<f64>().ok()?;
    let y = parts.next()?.trim().parse::<f64>().ok()?;
    let z = match parts.next() {
        Some(value) => value.trim().parse::<f64>().ok()?,
        None => 0.0,
    };
    if parts.next().is_some() {
        return None;
    }
    let point = Point3 { x, y, z };
    (point.x.is_finite() && point.y.is_finite() && point.z.is_finite()).then_some(point)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::HostController;

    fn controller() -> HostController {
        HostController::with_demo_document([800.0, 600.0]).expect("the demo document opens")
    }

    fn begin(controller: &mut HostController, kind: ApiPromptKind) -> ApiPrompt {
        let prompt = ApiPrompt::new("req-1", kind, None);
        controller
            .begin_api_prompt(prompt.clone())
            .expect("the prompt starts");
        prompt
    }

    fn results(controller: &mut HostController) -> Vec<ApiPromptResult> {
        controller.take_api_prompt_results()
    }

    #[test]
    fn a_second_prompt_is_refused_while_one_is_pending() {
        let mut controller = controller();
        begin(&mut controller, ApiPromptKind::Point);
        let error = controller
            .begin_api_prompt(ApiPrompt::new("req-2", ApiPromptKind::String, None))
            .expect_err("a second prompt is busy");
        assert!(
            matches!(&error, CadError::InvalidInput(marker) if marker == API_PROMPT_BUSY),
            "{error:?}"
        );
        assert_eq!(
            controller
                .api_prompt()
                .map(|prompt| prompt.request_id.as_str()),
            Some("req-1"),
            "the first prompt is untouched"
        );
    }

    #[test]
    fn a_full_result_queue_refuses_new_prompts() {
        let mut controller = controller();
        for index in 0..CAD_API_PROMPT_RESULT_CAP {
            controller.api_prompt_results.push(ApiPromptResult {
                request_id: format!("done-{index}"),
                reply: ApiPromptReply::Cancel,
            });
        }
        let error = controller
            .begin_api_prompt(ApiPrompt::new("req-1", ApiPromptKind::Point, None))
            .expect_err("the answer queue is full");
        assert!(
            matches!(&error, CadError::InvalidInput(marker) if marker == API_PROMPT_RESULTS_FULL),
            "{error:?}"
        );
    }

    #[test]
    fn a_point_prompt_completes_on_the_resolved_point() {
        let mut controller = controller();
        begin(&mut controller, ApiPromptKind::Point);
        controller.resolve_api_prompt(ApiPromptInput::Point(Point3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        }));
        assert!(!controller.api_prompt_pending());
        assert_eq!(
            results(&mut controller),
            vec![ApiPromptResult {
                request_id: "req-1".to_string(),
                reply: ApiPromptReply::Point(Point3 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                }),
            }]
        );
    }

    #[test]
    fn typed_text_parses_per_kind_and_reprompts_invalid_input() {
        let mut controller = controller();
        begin(&mut controller, ApiPromptKind::Double);
        controller.resolve_api_prompt(ApiPromptInput::Text("fast".to_string()));
        assert!(controller.api_prompt_pending(), "invalid input keeps it");
        assert_eq!(
            controller.api_prompt_error(),
            Some("invalid input: enter a number")
        );
        assert!(results(&mut controller).is_empty(), "nothing is answered");

        controller.resolve_api_prompt(ApiPromptInput::Text("2.5".to_string()));
        assert!(!controller.api_prompt_pending());
        assert_eq!(controller.api_prompt_error(), None, "the error clears");
        assert_eq!(
            results(&mut controller)[0].reply,
            ApiPromptReply::Number(2.5)
        );
    }

    #[test]
    fn an_integer_prompt_refuses_fractions_and_out_of_range_values() {
        let mut controller = controller();
        begin(&mut controller, ApiPromptKind::Integer);
        controller.resolve_api_prompt(ApiPromptInput::Text("1.5".to_string()));
        assert_eq!(
            controller.api_prompt_error(),
            Some("invalid input: enter a whole number")
        );
        controller.resolve_api_prompt(ApiPromptInput::Text("9007199254740993".to_string()));
        assert_eq!(
            controller.api_prompt_error(),
            Some("invalid input: enter a whole number"),
            "values beyond 2^53 cannot round-trip"
        );
        controller.resolve_api_prompt(ApiPromptInput::Text("-7".to_string()));
        assert_eq!(
            results(&mut controller)[0].reply,
            ApiPromptReply::Number(-7.0)
        );
    }

    #[test]
    fn a_keyword_wins_over_the_value_and_keeps_the_callers_spelling() {
        let mut controller = controller();
        let mut prompt = ApiPrompt::new("req-1", ApiPromptKind::String, None);
        prompt.keywords = vec!["Yes".to_string(), "No".to_string()];
        controller
            .begin_api_prompt(prompt)
            .expect("the prompt starts");
        controller.resolve_api_prompt(ApiPromptInput::Text("yEs".to_string()));
        assert_eq!(
            results(&mut controller)[0].reply,
            ApiPromptReply::Keyword("Yes".to_string())
        );
    }

    #[test]
    fn a_string_prompt_keeps_spaces_and_uses_the_default_on_enter() {
        let mut controller = controller();
        let mut prompt = ApiPrompt::new("req-1", ApiPromptKind::String, None);
        prompt.default = Some(ApiPromptDefault::String("plan A".to_string()));
        controller
            .begin_api_prompt(prompt)
            .expect("the prompt starts");
        controller.resolve_api_prompt(ApiPromptInput::Text(" two words ".to_string()));
        assert_eq!(
            results(&mut controller)[0].reply,
            ApiPromptReply::String(" two words ".to_string()),
            "a string answer is not trimmed"
        );

        let mut prompt = ApiPrompt::new("req-2", ApiPromptKind::String, None);
        prompt.default = Some(ApiPromptDefault::String("plan A".to_string()));
        controller
            .begin_api_prompt(prompt)
            .expect("the prompt starts");
        controller.resolve_api_prompt(ApiPromptInput::Confirm);
        assert_eq!(
            results(&mut controller)[0].reply,
            ApiPromptReply::String("plan A".to_string())
        );
    }

    #[test]
    fn enter_with_no_default_answers_none_and_escape_answers_cancel() {
        let mut controller = controller();
        begin(&mut controller, ApiPromptKind::Double);
        controller.resolve_api_prompt(ApiPromptInput::Confirm);
        assert_eq!(results(&mut controller)[0].reply, ApiPromptReply::None);

        begin(&mut controller, ApiPromptKind::Entity);
        controller.resolve_api_prompt(ApiPromptInput::Cancel);
        assert!(!controller.api_prompt_pending());
        assert_eq!(results(&mut controller)[0].reply, ApiPromptReply::Cancel);
    }

    #[test]
    fn an_entity_miss_reprompts_with_its_reject_sentence() {
        let mut controller = controller();
        let mut prompt = ApiPrompt::new("req-1", ApiPromptKind::Entity, None);
        prompt.reject_message = Some("Pick a line".to_string());
        controller
            .begin_api_prompt(prompt)
            .expect("the prompt starts");
        controller.resolve_api_prompt(ApiPromptInput::EntityMiss);
        assert!(controller.api_prompt_pending());
        assert_eq!(controller.api_prompt_error(), Some("Pick a line"));
    }

    #[test]
    fn without_a_prompt_the_input_belongs_to_the_normal_path() {
        let mut controller = controller();
        assert_eq!(
            controller.resolve_api_prompt(ApiPromptInput::Confirm),
            ApiPromptDisposition::NotPrompted
        );
    }

    #[test]
    fn select_all_runs_the_real_command_and_answers_its_ids() {
        let mut controller = controller();
        let ids = controller
            .plugin_select_all()
            .expect("the demo selects all");
        assert!(!ids.is_empty(), "the demo document has selectable geometry");
        let session_ids: Vec<EntityId> = controller
            .session
            .selection
            .refs()
            .iter()
            .map(|reference| reference.entity)
            .collect();
        assert_eq!(ids, session_ids, "the answer is the published selection");
    }
}
