use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    ptr::NonNull,
};

use super::wayland::WaylandGlSurface;
use super::{
    ClipboardRead, ClipboardWrite, KeyAction, Modifiers, MouseButton, MouseState, NativeWakeup,
    native_wakeup,
};

#[repr(C)]
struct RawSurface {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn gpui_ghostty_surface_linux_new(
        platform_userdata: *mut c_void,
        make_current: unsafe extern "C" fn(*mut c_void) -> bool,
        clear_current: unsafe extern "C" fn(*mut c_void),
        swap_buffers: unsafe extern "C" fn(*mut c_void),
        working_directory: *const c_char,
        command: *const c_char,
        scale_factor: f64,
        wakeup_userdata: *mut c_void,
        wakeup: unsafe extern "C" fn(*mut c_void),
    ) -> *mut RawSurface;
    fn gpui_ghostty_surface_linux_free(surface: *mut RawSurface);
    fn gpui_ghostty_surface_linux_tick(surface: *mut RawSurface);
    fn gpui_ghostty_surface_linux_is_alive(surface: *const RawSurface) -> bool;
    fn gpui_ghostty_surface_linux_take_clipboard_read(
        surface: *mut RawSurface,
        selection: *mut bool,
    ) -> *mut c_void;
    fn gpui_ghostty_surface_linux_complete_clipboard_read(
        surface: *mut RawSurface,
        request: *mut c_void,
        text: *const c_char,
    );
    fn gpui_ghostty_surface_linux_take_clipboard_write(
        surface: *mut RawSurface,
        selection: *mut bool,
    ) -> *mut c_char;
    fn gpui_ghostty_surface_linux_free_clipboard_write(text: *mut c_char);
    fn gpui_ghostty_surface_linux_set_size(
        surface: *mut RawSurface,
        width: u32,
        height: u32,
        scale_factor: f64,
    );
    fn gpui_ghostty_surface_linux_set_visible(surface: *mut RawSurface, visible: bool);
    fn gpui_ghostty_surface_linux_set_focus(surface: *mut RawSurface, focused: bool);
    fn gpui_ghostty_surface_linux_key(
        surface: *mut RawSurface,
        action: c_int,
        modifiers: c_int,
        consumed_modifiers: c_int,
        keycode: u32,
        text: *const c_char,
        unshifted_codepoint: u32,
    ) -> bool;
    fn gpui_ghostty_surface_linux_text(
        surface: *mut RawSurface,
        text: *const c_char,
        length: usize,
    );
    fn gpui_ghostty_surface_linux_mouse_position(
        surface: *mut RawSurface,
        x: f64,
        y: f64,
        modifiers: c_int,
    );
    fn gpui_ghostty_surface_linux_mouse_button(
        surface: *mut RawSurface,
        state: c_int,
        button: c_int,
        modifiers: c_int,
    );
    fn gpui_ghostty_surface_linux_mouse_scroll(
        surface: *mut RawSurface,
        x: f64,
        y: f64,
        modifiers: c_int,
    );
}

pub struct NativeSurface {
    raw: NonNull<RawSurface>,
    platform: Box<WaylandGlSurface>,
    wakeup: NativeWakeup,
    _working_directory: CString,
    _command: CString,
}

impl NativeSurface {
    pub fn new(
        display: Option<NonNull<c_void>>,
        parent_surface: NonNull<c_void>,
        scale_factor: f64,
        working_directory: CString,
        command: CString,
    ) -> Result<Self, String> {
        let display = display.ok_or_else(|| "Wayland display handle is unavailable".to_owned())?;
        let wakeup = NativeWakeup::new();
        let mut platform =
            WaylandGlSurface::new(display, parent_surface, scale_factor, wakeup.clone())?;
        let platform_ptr = (&mut *platform as *mut WaylandGlSurface).cast::<c_void>();
        // SAFETY: The boxed platform pointer and wakeup allocation stay stable until
        // after the C surface is freed.
        let raw = unsafe {
            gpui_ghostty_surface_linux_new(
                platform_ptr,
                make_current,
                clear_current,
                swap_buffers,
                working_directory.as_ptr(),
                command.as_ptr(),
                scale_factor,
                wakeup.userdata(),
                native_wakeup,
            )
        };
        let raw = NonNull::new(raw)
            .ok_or_else(|| "libghostty could not create a Wayland terminal surface".to_owned())?;
        Ok(Self {
            raw,
            platform,
            wakeup,
            _working_directory: working_directory,
            _command: command,
        })
    }

    pub fn wakeup(&self) -> NativeWakeup {
        self.wakeup.clone()
    }

    pub fn tick(&mut self) {
        self.platform.dispatch_pending();
        // SAFETY: `raw` is uniquely owned and Ghostty routes OpenGL work to this thread.
        unsafe { gpui_ghostty_surface_linux_tick(self.raw.as_ptr()) }
    }

    pub fn is_alive(&self) -> bool {
        unsafe { gpui_ghostty_surface_linux_is_alive(self.raw.as_ptr()) }
    }

    pub fn set_frame(&mut self, x: f64, y: f64, width: f64, height: f64, scale_factor: f64) {
        let Ok((physical_width, physical_height)) = self.platform.resize(
            x.round() as i32,
            y.round() as i32,
            width.round() as i32,
            height.round() as i32,
            scale_factor,
        ) else {
            return;
        };
        // SAFETY: Geometry and context updates completed before Ghostty observes the size.
        unsafe {
            gpui_ghostty_surface_linux_set_size(
                self.raw.as_ptr(),
                physical_width,
                physical_height,
                self.platform.scale(),
            )
        }
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.platform.set_visible(visible);
        unsafe { gpui_ghostty_surface_linux_set_visible(self.raw.as_ptr(), visible) }
    }

    pub fn set_focus(&mut self, focused: bool) {
        unsafe { gpui_ghostty_surface_linux_set_focus(self.raw.as_ptr(), focused) }
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
        unsafe {
            gpui_ghostty_surface_linux_key(
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
        unsafe {
            gpui_ghostty_surface_linux_text(self.raw.as_ptr(), text.as_ptr(), text.to_bytes().len())
        }
    }

    pub fn mouse_position(&mut self, x: f64, y: f64, modifiers: Modifiers) {
        unsafe {
            gpui_ghostty_surface_linux_mouse_position(self.raw.as_ptr(), x, y, modifiers.bits())
        }
    }

    pub fn mouse_button(&mut self, state: MouseState, button: MouseButton, modifiers: Modifiers) {
        unsafe {
            gpui_ghostty_surface_linux_mouse_button(
                self.raw.as_ptr(),
                state as c_int,
                button as c_int,
                modifiers.bits(),
            )
        }
    }

    pub fn mouse_scroll(&mut self, x: f64, y: f64, precision: bool) {
        unsafe {
            gpui_ghostty_surface_linux_mouse_scroll(self.raw.as_ptr(), x, y, i32::from(precision))
        }
    }

    pub fn take_clipboard_read(&mut self) -> Option<ClipboardRead> {
        let mut selection = false;
        let request = unsafe {
            gpui_ghostty_surface_linux_take_clipboard_read(self.raw.as_ptr(), &mut selection)
        };
        Some(ClipboardRead {
            selection,
            request: NonNull::new(request)?,
        })
    }

    pub fn complete_clipboard_read(&mut self, request: ClipboardRead, text: &CStr) {
        unsafe {
            gpui_ghostty_surface_linux_complete_clipboard_read(
                self.raw.as_ptr(),
                request.request.as_ptr(),
                text.as_ptr(),
            )
        }
    }

    pub fn take_clipboard_write(&mut self) -> Option<ClipboardWrite> {
        let mut selection = false;
        let text = unsafe {
            gpui_ghostty_surface_linux_take_clipboard_write(self.raw.as_ptr(), &mut selection)
        };
        let text = NonNull::new(text)?;
        let value = unsafe { CStr::from_ptr(text.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        unsafe { gpui_ghostty_surface_linux_free_clipboard_write(text.as_ptr()) };
        Some(ClipboardWrite {
            selection,
            text: value,
        })
    }
}

impl Drop for NativeSurface {
    fn drop(&mut self) {
        // SAFETY: C teardown makes the context current and completes before platform drop.
        unsafe { gpui_ghostty_surface_linux_free(self.raw.as_ptr()) }
    }
}

unsafe extern "C" fn make_current(userdata: *mut c_void) -> bool {
    if userdata.is_null() {
        return false;
    }
    // SAFETY: The pointer targets the stable boxed platform owned by NativeSurface.
    unsafe { &*(userdata.cast::<WaylandGlSurface>()) }
        .make_current()
        .is_ok()
}

unsafe extern "C" fn clear_current(userdata: *mut c_void) {
    if !userdata.is_null() {
        unsafe { &*(userdata.cast::<WaylandGlSurface>()) }.clear_current();
    }
}

unsafe extern "C" fn swap_buffers(userdata: *mut c_void) {
    if !userdata.is_null() {
        unsafe { &*(userdata.cast::<WaylandGlSurface>()) }.swap_buffers();
    }
}
