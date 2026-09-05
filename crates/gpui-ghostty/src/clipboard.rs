use std::sync::Arc;

/// A clipboard operation for which Ghostty requires application approval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardOperation {
    /// A paste rejected by Ghostty's unsafe-paste protection.
    Paste,
    /// A terminal program requesting clipboard contents using OSC 52.
    Read,
    /// A terminal program writing the clipboard when configured to ask.
    Write,
}

/// Data awaiting approval. The text may contain sensitive clipboard contents.
pub struct ClipboardApproval<'a> {
    pub operation: ClipboardOperation,
    pub text: &'a str,
}

/// Synchronous approval policy. Return `true` to accept the operation.
///
/// Called at the native boundary: do not block, re-enter the terminal, or open
/// a modal event loop. A missing callback or a panic denies the request. Only
/// operations requiring confirmation invoke this policy; Ghostty's explicit
/// `allow`/`deny` configuration continues to apply.
///
/// Denied reads return an empty clipboard response; denied pastes insert nothing.
pub type ClipboardApprovalCallback = Arc<dyn Fn(ClipboardApproval<'_>) -> bool + Send + Sync>;
