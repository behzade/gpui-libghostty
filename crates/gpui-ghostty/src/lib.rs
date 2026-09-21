//! Native libghostty terminal component for GPUI.
//!
//! Rendering uses Ghostty's Metal embedded surface on macOS and a native
//! Wayland subsurface backed by its OpenGL renderer on Linux.
//!
//! Call `gpui_libghostty::bind_gpui!(gpui)` once in a shared module to define
//! `Terminal` against your application's GPUI dependency. Import the options
//! and configuration types from this crate as usual.

mod adapter;
mod clipboard;
mod native;
mod terminal;

pub use clipboard::{ClipboardApproval, ClipboardApprovalCallback, ClipboardOperation};
pub use terminal::{TerminalColor, TerminalConfiguration, TerminalOptions, TerminalTheme};

/// Implementation details used by `bind_gpui!`; not a standalone public API.
#[doc(hidden)]
pub mod __private {
    pub use crate::native::{KeyAction, Modifiers, MouseButton, MouseState, NativeSurface};
    pub use crate::terminal::spawn_surface;
}
