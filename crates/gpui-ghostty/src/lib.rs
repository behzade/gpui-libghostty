//! Native libghostty terminal component for GPUI.
//!
//! The renderer currently uses Ghostty's macOS Metal surface API. Other targets
//! compile, but [`Terminal::spawn`] returns an unsupported-platform error.

mod native;
mod terminal;

pub use terminal::{Terminal, TerminalOptions};
