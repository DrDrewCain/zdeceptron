//! What a pass in this crate says when it refuses a program.
//!
//! Spec §7.3 makes diagnostics a primary deliverable, and §17.2.10 and
//! §17.3.8 both require more than a message and a caret: a cross-region
//! rejection must print the path from the root, and an information-flow
//! rejection must print the path along which the value would have escaped.
//! So an error here is a message, a point, an *ordered list of further
//! points*, and a repair.

use zdc_lexer::Span;

/// Whether a finding stops the build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One finding, with its spec code.
///
/// The code is carried separately from the message so a test can assert
/// on `E0301` without asserting on prose, which is the thing most likely
/// to be improved later.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphError {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub span: Span,
    /// The path, in reading order. §17.2.10's "reached: hourly → ingest →
    /// name" and §17.3.8's escape trace are both this.
    pub notes: Vec<(Span, String)>,
    pub help: Option<String>,
}

impl GraphError {
    pub fn new(code: &'static str, message: impl Into<String>, span: Span) -> GraphError {
        GraphError {
            code,
            severity: Severity::Error,
            message: message.into(),
            span,
            notes: Vec::new(),
            help: None,
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>, span: Span) -> GraphError {
        GraphError {
            severity: Severity::Warning,
            ..GraphError::new(code, message, span)
        }
    }

    pub fn with_notes(mut self, notes: Vec<(Span, String)>) -> GraphError {
        self.notes = notes;
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> GraphError {
        self.help = Some(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// The message as a reader sees it, with the code in front.
    pub fn rendered_message(&self) -> String {
        match self.severity {
            Severity::Error => format!("error[{}]: {}", self.code, self.message),
            Severity::Warning => format!("warning[{}]: {}", self.code, self.message),
        }
    }
}
