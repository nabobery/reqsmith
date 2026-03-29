use std::path::PathBuf;

use crate::core::models::{AssertionReport, CollectionNode, RequestDocument, ResponseArtifact};

/// Shared action vocabulary for state transitions across the application.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum Action {
    Tick,
    Render,
    Quit,
    FocusNext,
    FocusPrev,
    Resize(u16, u16),
    Error(String),

    // Collection actions
    CollectionsDiscovered(Vec<CollectionNode>),
    SelectRequest(PathBuf),
    RequestLoaded(Box<RequestDocument>),
    RefreshCollections,

    // Editor actions
    SaveRequest,
    RequestSaved(PathBuf),

    // Execution actions
    SendRequest,
    CancelRequest,
    RequestCompleted(Box<ResponseArtifact>, Box<AssertionReport>),
    RequestFailed(String),
    RequestCancelled,
    SaveResponseBody,

    // Status
    StatusMessage(String),
}

/// Identifies which pane currently holds focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    Collections,
    RequestEditor,
    ResponseViewer,
}

impl FocusTarget {
    pub fn next(self) -> Self {
        match self {
            Self::Collections => Self::RequestEditor,
            Self::RequestEditor => Self::ResponseViewer,
            Self::ResponseViewer => Self::Collections,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Collections => Self::ResponseViewer,
            Self::RequestEditor => Self::Collections,
            Self::ResponseViewer => Self::RequestEditor,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Collections => "Collections",
            Self::RequestEditor => "Request",
            Self::ResponseViewer => "Response",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_target_next_cycles_through_all_variants() {
        let start = FocusTarget::Collections;
        let second = start.next();
        let third = second.next();
        let wrapped = third.next();

        assert_eq!(second, FocusTarget::RequestEditor);
        assert_eq!(third, FocusTarget::ResponseViewer);
        assert_eq!(wrapped, FocusTarget::Collections);
    }

    #[test]
    fn focus_target_prev_cycles_through_all_variants() {
        let start = FocusTarget::Collections;
        let second = start.prev();
        let third = second.prev();
        let wrapped = third.prev();

        assert_eq!(second, FocusTarget::ResponseViewer);
        assert_eq!(third, FocusTarget::RequestEditor);
        assert_eq!(wrapped, FocusTarget::Collections);
    }

    #[test]
    fn focus_target_next_and_prev_are_inverse() {
        for target in [
            FocusTarget::Collections,
            FocusTarget::RequestEditor,
            FocusTarget::ResponseViewer,
        ] {
            assert_eq!(target.next().prev(), target);
            assert_eq!(target.prev().next(), target);
        }
    }
}
