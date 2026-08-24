//! Safe, narrow Rust ownership wrapper for Ghostty's internal macOS surface API.

use std::{
    ffi::{CStr, c_void},
    ptr::NonNull,
};

#[cfg(target_os = "macos")]
use std::{
    ffi::{c_char, c_int},
    marker::PhantomData,
    rc::Rc,
};

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    #[repr(C)]
    struct RawSurface {
        _private: [u8; 0],
    }

    unsafe extern "C" {
        fn gpui_ghostty_surface_new(
            parent_view: *mut c_void,
            working_directory: *const c_char,
            command: *const c_char,
        ) -> *mut RawSurface;
        fn gpui_ghostty_surface_free(surface: *mut RawSurface);
        fn gpui_ghostty_surface_tick(surface: *mut RawSurface);
        fn gpui_ghostty_surface_needs_tick(surface: *const RawSurface) -> bool;
        fn gpui_ghostty_surface_is_alive(surface: *const RawSurface) -> bool;
        fn gpui_ghostty_surface_set_frame(
            surface: *mut RawSurface,
            x: f64,
            y: f64,
            width: f64,
            height: f64,
        );
        fn gpui_ghostty_surface_set_visible(surface: *mut RawSurface, visible: bool);
        fn gpui_ghostty_surface_set_focus(surface: *mut RawSurface, focused: bool);
        fn gpui_ghostty_surface_key(
            surface: *mut RawSurface,
            action: c_int,
            modifiers: c_int,
            consumed_modifiers: c_int,
            keycode: u32,
            text: *const c_char,
            unshifted_codepoint: u32,
        ) -> bool;
        fn gpui_ghostty_surface_text(surface: *mut RawSurface, text: *const c_char, length: usize);
        fn gpui_ghostty_surface_mouse_position(
            surface: *mut RawSurface,
            x: f64,
            y: f64,
            modifiers: c_int,
        );
        fn gpui_ghostty_surface_mouse_button(
            surface: *mut RawSurface,
            state: c_int,
            button: c_int,
            modifiers: c_int,
        );
        fn gpui_ghostty_surface_mouse_scroll(
            surface: *mut RawSurface,
            x: f64,
            y: f64,
            modifiers: c_int,
        );
    }

    pub struct NativeSurface {
        raw: NonNull<RawSurface>,
        _main_thread: PhantomData<Rc<()>>,
    }

    impl NativeSurface {
        /// Creates a Ghostty-rendered child view attached to an AppKit `NSView`.
        ///
        /// The caller must invoke this on the AppKit main thread and keep the parent
        /// view alive until this value is dropped.
        pub fn new(
            parent_view: NonNull<c_void>,
            working_directory: &CStr,
            command: &CStr,
        ) -> Result<Self, &'static str> {
            // SAFETY: The C shim validates creation failures. The parent pointer and
            // main-thread lifetime requirements are this method's documented boundary.
            let raw = unsafe {
                gpui_ghostty_surface_new(
                    parent_view.as_ptr(),
                    working_directory.as_ptr(),
                    command.as_ptr(),
                )
            };
            let raw = NonNull::new(raw).ok_or("libghostty could not create a terminal surface")?;
            Ok(Self {
                raw,
                _main_thread: PhantomData,
            })
        }

        pub fn tick(&mut self) {
            // SAFETY: `raw` is owned by this value and calls stay on the main thread.
            unsafe { gpui_ghostty_surface_tick(self.raw.as_ptr()) }
        }

        pub fn needs_tick(&self) -> bool {
            // SAFETY: `raw` remains valid for this value's lifetime.
            unsafe { gpui_ghostty_surface_needs_tick(self.raw.as_ptr()) }
        }

        pub fn is_alive(&self) -> bool {
            // SAFETY: `raw` remains valid for this value's lifetime.
            unsafe { gpui_ghostty_surface_is_alive(self.raw.as_ptr()) }
        }

        pub fn set_frame(&mut self, x: f64, y: f64, width: f64, height: f64) {
            // SAFETY: `raw` is valid and geometry values cross the C boundary by value.
            unsafe { gpui_ghostty_surface_set_frame(self.raw.as_ptr(), x, y, width, height) }
        }

        pub fn set_visible(&mut self, visible: bool) {
            // SAFETY: `raw` is valid and this is called from the AppKit main thread.
            unsafe { gpui_ghostty_surface_set_visible(self.raw.as_ptr(), visible) }
        }

        pub fn set_focus(&mut self, focused: bool) {
            // SAFETY: `raw` is valid and this is called from the AppKit main thread.
            unsafe { gpui_ghostty_surface_set_focus(self.raw.as_ptr(), focused) }
        }

        pub fn key(
            &mut self,
            action: KeyAction,
            modifiers: Modifiers,
            consumed_modifiers: Modifiers,
            keycode: u32,
            text: Option<&CStr>,
            unshifted_codepoint: u32,
        ) -> bool {
            // SAFETY: Optional text remains valid for the duration of the call and
            // all integer values match the C shim's stable adapter ABI.
            unsafe {
                gpui_ghostty_surface_key(
                    self.raw.as_ptr(),
                    action as c_int,
                    modifiers.bits(),
                    consumed_modifiers.bits(),
                    keycode,
                    text.map_or(std::ptr::null(), CStr::as_ptr),
                    unshifted_codepoint,
                )
            }
        }

        pub fn text(&mut self, text: &CStr) {
            // SAFETY: The bytes remain valid for the duration of the call.
            unsafe {
                gpui_ghostty_surface_text(self.raw.as_ptr(), text.as_ptr(), text.to_bytes().len())
            }
        }

        pub fn mouse_position(&mut self, x: f64, y: f64, modifiers: Modifiers) {
            // SAFETY: Values cross the adapter ABI by value.
            unsafe {
                gpui_ghostty_surface_mouse_position(self.raw.as_ptr(), x, y, modifiers.bits())
            }
        }

        pub fn mouse_button(
            &mut self,
            state: MouseState,
            button: MouseButton,
            modifiers: Modifiers,
        ) {
            // SAFETY: Values cross the adapter ABI by value.
            unsafe {
                gpui_ghostty_surface_mouse_button(
                    self.raw.as_ptr(),
                    state as c_int,
                    button as c_int,
                    modifiers.bits(),
                )
            }
        }

        pub fn mouse_scroll(&mut self, x: f64, y: f64, precision: bool) {
            // Bit zero is Ghostty's high-precision scroll flag. Momentum is left
            // unset because GPUI does not expose AppKit's momentum phase directly.
            let scroll_flags = i32::from(precision);
            // SAFETY: Values cross the adapter ABI by value.
            unsafe { gpui_ghostty_surface_mouse_scroll(self.raw.as_ptr(), x, y, scroll_flags) }
        }
    }

    impl Drop for NativeSurface {
        fn drop(&mut self) {
            // SAFETY: This value uniquely owns `raw` and drops on the AppKit main thread.
            unsafe { gpui_ghostty_surface_free(self.raw.as_ptr()) }
        }
    }
}

#[cfg(target_os = "macos")]
pub use platform::NativeSurface;

#[cfg(not(target_os = "macos"))]
pub struct NativeSurface;

#[cfg(not(target_os = "macos"))]
impl NativeSurface {
    pub fn new(
        _parent_view: NonNull<c_void>,
        _working_directory: &CStr,
        _command: &CStr,
    ) -> Result<Self, &'static str> {
        Err("libghostty native surfaces are only available on macOS")
    }

    pub fn tick(&mut self) {}
    pub fn needs_tick(&self) -> bool {
        false
    }
    pub fn is_alive(&self) -> bool {
        false
    }
    pub fn set_frame(&mut self, _x: f64, _y: f64, _width: f64, _height: f64) {}
    pub fn set_visible(&mut self, _visible: bool) {}
    pub fn set_focus(&mut self, _focused: bool) {}
    pub fn key(
        &mut self,
        _action: KeyAction,
        _modifiers: Modifiers,
        _consumed_modifiers: Modifiers,
        _keycode: u32,
        _text: Option<&CStr>,
        _unshifted_codepoint: u32,
    ) -> bool {
        false
    }
    pub fn text(&mut self, _text: &CStr) {}
    pub fn mouse_position(&mut self, _x: f64, _y: f64, _modifiers: Modifiers) {}
    pub fn mouse_button(
        &mut self,
        _state: MouseState,
        _button: MouseButton,
        _modifiers: Modifiers,
    ) {
    }
    pub fn mouse_scroll(&mut self, _x: f64, _y: f64, _precision: bool) {}
}

#[derive(Clone, Copy)]
#[repr(i32)]
pub enum KeyAction {
    Release = 0,
    Press = 1,
    Repeat = 2,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modifiers(i32);

impl Modifiers {
    pub const SHIFT: Self = Self(1 << 0);
    pub const CONTROL: Self = Self(1 << 1);
    pub const ALT: Self = Self(1 << 2);
    pub const SUPER: Self = Self(1 << 3);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    const fn bits(self) -> i32 {
        self.0
    }
}

#[derive(Clone, Copy)]
#[repr(i32)]
pub enum MouseState {
    Release = 0,
    Press = 1,
}

#[derive(Clone, Copy)]
#[repr(i32)]
pub enum MouseButton {
    Unknown = 0,
    Left = 1,
    Right = 2,
    Middle = 3,
}
