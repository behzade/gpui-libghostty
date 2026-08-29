//! Safe, narrow Rust ownership wrappers for Ghostty's native render surfaces.

use std::{
    ffi::{CString, c_void},
    ptr::NonNull,
    sync::Arc,
};

#[cfg(not(target_os = "linux"))]
use std::ffi::CStr;

use async_channel::{Receiver, Sender};

#[derive(Clone)]
pub struct NativeWakeup {
    sender: Arc<Sender<()>>,
    receiver: Receiver<()>,
}

impl NativeWakeup {
    fn new() -> Self {
        let (sender, receiver) = async_channel::bounded(1);
        Self {
            sender: Arc::new(sender),
            receiver,
        }
    }

    pub async fn wait(&self) {
        let _ = self.receiver.recv().await;
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn signal(&self) {
        let _ = self.sender.try_send(());
    }

    fn userdata(&self) -> *mut c_void {
        Arc::as_ptr(&self.sender).cast_mut().cast()
    }
}

unsafe extern "C" fn native_wakeup(userdata: *mut c_void) {
    let Some(sender) = NonNull::new(userdata.cast::<Sender<()>>()) else {
        return;
    };
    // SAFETY: NativeSurface keeps the Arc allocation alive until native teardown completes.
    let _ = unsafe { sender.as_ref() }.try_send(());
}

#[derive(Clone, Copy, PartialEq)]
struct NativeFrame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale_factor: f64,
}

impl NativeFrame {
    fn new(x: f64, y: f64, width: f64, height: f64, scale_factor: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
            scale_factor,
        }
    }
}

#[derive(Default)]
struct NativeSurfaceState {
    frame: Option<NativeFrame>,
    visible: bool,
}

pub(crate) struct NativeSnapshot {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bgra: Vec<u8>,
}

impl NativeSnapshot {
    unsafe fn copy_from_raw(
        pixels: *const u8,
        width: u32,
        height: u32,
        length: usize,
        bottom_up: bool,
    ) -> Result<Self, String> {
        let row_length = usize::try_from(width)
            .ok()
            .filter(|width| *width > 0)
            .and_then(|width| width.checked_mul(4));
        let expected = row_length.and_then(|row| {
            usize::try_from(height)
                .ok()
                .filter(|height| *height > 0)
                .and_then(|height| row.checked_mul(height))
        });
        let (Some(pixels), Some(row_length), Some(expected)) =
            (NonNull::new(pixels.cast_mut()), row_length, expected)
        else {
            return Err("native terminal snapshot has invalid dimensions".to_owned());
        };
        if expected != length {
            return Err("native terminal snapshot has invalid dimensions".to_owned());
        }

        // SAFETY: The caller guarantees that `pixels` references `length` readable bytes.
        let mut bgra = unsafe { std::slice::from_raw_parts(pixels.as_ptr(), length) }.to_vec();
        if bottom_up {
            flip_bgra_rows(&mut bgra, row_length);
        }
        Ok(Self {
            width,
            height,
            bgra,
        })
    }
}

fn flip_bgra_rows(pixels: &mut [u8], row_length: usize) {
    let rows = pixels.len() / row_length;
    for row in 0..rows / 2 {
        let opposite = rows - row - 1;
        let (before, after) = pixels.split_at_mut(opposite * row_length);
        before[row * row_length..(row + 1) * row_length].swap_with_slice(&mut after[..row_length]);
    }
}

impl NativeSurfaceState {
    fn frame_changed(&self, frame: NativeFrame) -> bool {
        self.frame != Some(frame)
    }

    fn commit_visible_frame(&mut self, frame: NativeFrame) {
        self.frame = Some(frame);
        self.visible = true;
    }

    fn update_visibility(&mut self, visible: bool) -> bool {
        if self.visible == visible {
            return false;
        }
        self.visible = visible;
        true
    }
}

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
            load_user_config: bool,
            theme_config_path: *const c_char,
            wakeup_userdata: *mut c_void,
            wakeup: unsafe extern "C" fn(*mut c_void),
        ) -> *mut RawSurface;
        fn gpui_ghostty_surface_free(surface: *mut RawSurface);
        fn gpui_ghostty_surface_tick(surface: *mut RawSurface);
        fn gpui_ghostty_surface_is_alive(surface: *const RawSurface) -> bool;
        fn gpui_ghostty_surface_snapshot(
            surface: *mut RawSurface,
            pixels: *mut *mut u8,
            width: *mut u32,
            height: *mut u32,
            length: *mut usize,
        ) -> bool;
        fn gpui_ghostty_surface_snapshot_free(pixels: *mut u8);
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
        wakeup: NativeWakeup,
        state: NativeSurfaceState,
        _working_directory: CString,
        _command: CString,
        _main_thread: PhantomData<Rc<()>>,
    }

    impl NativeSurface {
        /// Creates a Ghostty-rendered child view attached to an AppKit `NSView`.
        ///
        /// The caller must invoke this on the AppKit main thread and keep the parent
        /// view alive until this value is dropped.
        pub fn new(
            _display: Option<NonNull<c_void>>,
            parent_view: NonNull<c_void>,
            _scale_factor: f64,
            working_directory: CString,
            command: CString,
            load_user_config: bool,
            theme_config_path: Option<&CStr>,
        ) -> Result<Self, &'static str> {
            let wakeup = NativeWakeup::new();
            // SAFETY: The C shim validates creation failures. The parent pointer and
            // main-thread lifetime requirements are this method's documented boundary.
            let raw = unsafe {
                gpui_ghostty_surface_new(
                    parent_view.as_ptr(),
                    working_directory.as_ptr(),
                    command.as_ptr(),
                    load_user_config,
                    theme_config_path.map_or(std::ptr::null(), CStr::as_ptr),
                    wakeup.userdata(),
                    native_wakeup,
                )
            };
            let raw = NonNull::new(raw).ok_or("libghostty could not create a terminal surface")?;
            Ok(Self {
                raw,
                wakeup,
                state: NativeSurfaceState::default(),
                _working_directory: working_directory,
                _command: command,
                _main_thread: PhantomData,
            })
        }

        pub fn wakeup(&self) -> NativeWakeup {
            self.wakeup.clone()
        }

        pub fn tick(&mut self) {
            // SAFETY: `raw` is owned by this value and calls stay on the main thread.
            unsafe { gpui_ghostty_surface_tick(self.raw.as_ptr()) }
        }

        pub fn is_alive(&self) -> bool {
            // SAFETY: `raw` remains valid for this value's lifetime.
            unsafe { gpui_ghostty_surface_is_alive(self.raw.as_ptr()) }
        }

        pub fn snapshot(&mut self) -> Result<NativeSnapshot, String> {
            let mut pixels = std::ptr::null_mut();
            let mut width = 0;
            let mut height = 0;
            let mut length = 0;
            // SAFETY: The shim initializes all outputs and returns a malloc-owned
            // buffer which remains valid until the matching free call below.
            let captured = unsafe {
                gpui_ghostty_surface_snapshot(
                    self.raw.as_ptr(),
                    &mut pixels,
                    &mut width,
                    &mut height,
                    &mut length,
                )
            };
            if !captured {
                return Err("capture native terminal frame".to_owned());
            }
            // SAFETY: A successful shim call returns `length` readable bytes in `pixels`.
            let result =
                unsafe { NativeSnapshot::copy_from_raw(pixels, width, height, length, false) };
            // SAFETY: `pixels` is either null or the allocation returned by the shim.
            unsafe { gpui_ghostty_surface_snapshot_free(pixels) }
            result
        }

        pub fn set_frame(&mut self, x: f64, y: f64, width: f64, height: f64, scale_factor: f64) {
            let frame = NativeFrame::new(x, y, width, height, scale_factor);
            if !self.state.frame_changed(frame) {
                self.set_visible(true);
                return;
            }
            // SAFETY: `raw` is valid and geometry values cross the C boundary by value.
            unsafe { gpui_ghostty_surface_set_frame(self.raw.as_ptr(), x, y, width, height) }
            self.state.commit_visible_frame(frame);
        }

        pub fn set_visible(&mut self, visible: bool) {
            if self.state.update_visibility(visible) {
                // SAFETY: `raw` is valid and this is called from the AppKit main thread.
                unsafe { gpui_ghostty_surface_set_visible(self.raw.as_ptr(), visible) }
            }
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

        pub fn take_clipboard_read(&mut self) -> Option<ClipboardRead> {
            None
        }

        pub fn complete_clipboard_read(&mut self, _request: ClipboardRead, _text: &CStr) {}

        pub fn take_clipboard_write(&mut self) -> Option<ClipboardWrite> {
            None
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

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
pub use linux::NativeSurface;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub struct NativeSurface;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl NativeSurface {
    pub fn new(
        _display: Option<NonNull<c_void>>,
        _parent_view: NonNull<c_void>,
        _scale_factor: f64,
        _working_directory: CString,
        _command: CString,
        _load_user_config: bool,
        _theme_config_path: Option<&CStr>,
    ) -> Result<Self, &'static str> {
        Err("libghostty native surfaces require macOS or Wayland")
    }

    pub fn wakeup(&self) -> NativeWakeup {
        NativeWakeup::new()
    }
    pub fn tick(&mut self) {}
    pub fn is_alive(&self) -> bool {
        false
    }
    pub fn snapshot(&mut self) -> Result<NativeSnapshot, String> {
        Err("native terminal snapshots require macOS or Wayland".to_owned())
    }
    pub fn set_frame(&mut self, _x: f64, _y: f64, _width: f64, _height: f64, _scale_factor: f64) {}
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
    pub fn take_clipboard_read(&mut self) -> Option<ClipboardRead> {
        None
    }
    pub fn complete_clipboard_read(&mut self, _request: ClipboardRead, _text: &CStr) {}
    pub fn take_clipboard_write(&mut self) -> Option<ClipboardWrite> {
        None
    }
}

pub struct ClipboardRead {
    pub selection: bool,
    #[allow(dead_code)]
    pub request: NonNull<c_void>,
}

pub struct ClipboardWrite {
    pub selection: bool,
    pub text: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_wakeup_coalesces_duplicate_signals() {
        let wakeup = NativeWakeup::new();
        wakeup.signal();
        wakeup.signal();

        assert_eq!(wakeup.receiver.try_recv(), Ok(()));
        assert_eq!(
            wakeup.receiver.try_recv(),
            Err(async_channel::TryRecvError::Empty)
        );
    }
}
