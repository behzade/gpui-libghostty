//! Native libghostty terminal component for GPUI.
//!
//! Rendering uses Ghostty's Metal embedded surface on macOS and a native
//! Wayland subsurface backed by its OpenGL renderer on Linux.

mod native;
mod terminal;

pub use terminal::{Terminal, TerminalOptions};
