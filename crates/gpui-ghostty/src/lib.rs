//! Native libghostty terminal component for GPUI.
//!
//! Rendering uses Ghostty's Metal embedded surface on macOS and a native
//! Wayland subsurface backed by its OpenGL renderer on Linux.

mod clipboard;
mod native;
mod terminal;

pub use clipboard::{ClipboardApproval, ClipboardApprovalCallback, ClipboardOperation};
pub use terminal::{
    Terminal, TerminalColor, TerminalConfiguration, TerminalOptions, TerminalTheme,
};
