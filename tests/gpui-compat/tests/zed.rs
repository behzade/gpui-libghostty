//! Exercise the shared consumer against the pinned Zed checkout.
#![forbid(unsafe_code)]

use editor_library as editor_core;
use gpui as ui;
use terminal_library as terminal_core;

#[path = "../src/bindings.rs"]
mod bindings;
