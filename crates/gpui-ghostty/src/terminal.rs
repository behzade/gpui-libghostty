//! GPUI-independent terminal configuration and input translation.
use crate::clipboard::ClipboardApprovalCallback;
use crate::native::{KeyAction, Modifiers, NativeSurface};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use std::{
    ffi::{CString, c_void},
    fmt,
    io::Write as _,
    path::PathBuf,
    ptr::NonNull,
};

/// An opaque terminal color without an alpha channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl TerminalColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl fmt::Display for TerminalColor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// Colors applied to a terminal before its process starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalTheme {
    pub background: TerminalColor,
    pub foreground: TerminalColor,
    pub palette: [TerminalColor; 16],
}

impl TerminalTheme {
    pub const fn new(
        background: TerminalColor,
        foreground: TerminalColor,
        palette: [TerminalColor; 16],
    ) -> Self {
        Self {
            background,
            foreground,
            palette,
        }
    }
}

/// Selects the source of terminal configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum TerminalConfiguration {
    /// Use Ghostty's built-in defaults without reading user configuration.
    #[default]
    Default,
    /// Load Ghostty's default and recursively referenced user configuration files.
    UserDefault,
    /// Use built-in defaults with application-supplied colors.
    Custom(TerminalTheme),
    /// Load user configuration, then override its colors with application values.
    UserDefaultWithOverride(TerminalTheme),
}

/// Configuration for a terminal process rendered by libghostty.
#[non_exhaustive]
pub struct TerminalOptions {
    pub command: String,
    pub working_directory: PathBuf,
    pub focus_on_spawn: bool,
    pub configuration: TerminalConfiguration,
    /// Approval policy for protected clipboard operations; absent means deny.
    pub clipboard_approval: Option<ClipboardApprovalCallback>,
}

impl TerminalOptions {
    pub fn new(command: impl Into<String>, working_directory: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            working_directory: working_directory.into(),
            focus_on_spawn: true,
            configuration: TerminalConfiguration::Default,
            clipboard_approval: None,
        }
    }
}

fn theme_config_contents(theme: &TerminalTheme) -> String {
    let palette = theme
        .palette
        .iter()
        .enumerate()
        .map(|(index, color)| format!("palette = {index}=#{color}\n"))
        .collect::<String>();
    format!(
        "background = #{}\nforeground = #{}\n{}palette-generate = true\n",
        theme.background, theme.foreground, palette
    )
}

fn write_theme_config(theme: &TerminalTheme) -> Result<tempfile::NamedTempFile, String> {
    let mut file = tempfile::NamedTempFile::new()
        .map_err(|error| format!("create temporary Ghostty theme: {error}"))?;
    file.write_all(theme_config_contents(theme).as_bytes())
        .map_err(|error| format!("write temporary Ghostty theme: {error}"))?;
    Ok(file)
}

/// Creates the native child used by the generated adapter.
///
/// # Safety
/// Call on the window's UI thread. The parent window and display must outlive
/// the surface, and the surface must be used and dropped on that same thread.
pub unsafe fn spawn_surface(
    options: TerminalOptions,
    window: &(impl raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle),
    scale_factor: f64,
) -> Result<NativeSurface, String> {
    let TerminalOptions {
        command,
        working_directory,
        focus_on_spawn: _,
        configuration,
        clipboard_approval,
    } = options;
    let working_directory =
        CString::new(working_directory.to_string_lossy().as_bytes()).map_err(|_| {
            format!(
                "terminal working directory contains a NUL byte: {}",
                working_directory.display()
            )
        })?;
    let command =
        CString::new(command).map_err(|_| "terminal command contains a NUL byte".to_owned())?;
    let (load_user_config, theme_config) = match configuration {
        TerminalConfiguration::Default => (false, None),
        TerminalConfiguration::UserDefault => (true, None),
        TerminalConfiguration::Custom(theme) => (false, Some(write_theme_config(&theme)?)),
        TerminalConfiguration::UserDefaultWithOverride(theme) => {
            (true, Some(write_theme_config(&theme)?))
        }
    };
    let theme_config_path = theme_config
        .as_ref()
        .map(|file| CString::new(file.path().to_string_lossy().as_bytes()))
        .transpose()
        .map_err(|_| "temporary Ghostty theme path contains a NUL byte".to_owned())?;
    let native_window = native_window(window)?;
    let surface = unsafe {
        NativeSurface::new(
            native_window.display,
            native_window.surface,
            scale_factor,
            working_directory,
            command,
            load_user_config,
            theme_config_path.as_deref(),
        )
    }
    .map_err(|error| format!("initialize libghostty: {error}"))?;
    surface.wakeup().init_clipboard_approval(clipboard_approval);
    Ok(surface)
}

impl NativeSurface {
    pub fn send_key(
        &mut self,
        action: KeyAction,
        key: &str,
        text: Option<&str>,
        modifiers: Modifiers,
    ) {
        let Some(key) = native_key(key) else {
            if matches!(action, KeyAction::Press | KeyAction::Repeat)
                && !modifiers.contains(Modifiers::CONTROL)
                && !modifiers.contains(Modifiers::ALT)
                && !modifiers.contains(Modifiers::SUPER)
                && let Some(text) = text
                && let Ok(text) = CString::new(text)
            {
                self.text(&text);
            }
            return;
        };
        let text = text.and_then(|text| CString::new(text).ok());
        let (active_modifiers, consumed_modifiers) =
            key_modifiers(modifiers, key.implied_shift, text.is_some());
        let _ = self.key(
            action,
            active_modifiers,
            consumed_modifiers,
            key.keycode,
            text.as_deref(),
            key.unshifted_codepoint,
        );
    }

    pub fn snapshot_frame(&mut self) -> Result<image::Frame, String> {
        let snapshot = self.snapshot()?;
        let image = image::RgbaImage::from_raw(snapshot.width, snapshot.height, snapshot.bgra)
            .ok_or_else(|| "native terminal snapshot has an invalid byte length".to_owned())?;
        Ok(image::Frame::new(image))
    }

    /// Services one read without exposing the native request. The callback gets
    /// the selection flag; completion always uses the originating surface.
    ///
    /// Callers cannot obtain or forge a raw request:
    /// ```compile_fail
    /// use gpui_libghostty::__private::NativeSurface;
    /// fn forge(surface: &mut NativeSurface) {
    ///     let mut request = surface.take_clipboard_read().unwrap();
    ///     request.request = std::ptr::NonNull::dangling();
    /// }
    /// ```
    pub fn service_clipboard_read(&mut self, read: impl FnOnce(bool) -> String) {
        let Some(request) = self.take_clipboard_read() else {
            return;
        };
        let text = read(request.selection).replace('\0', "�");
        if let Ok(text) = CString::new(text) {
            self.complete_clipboard_read(request, &text);
        }
    }
}

struct NativeWindow {
    display: Option<NonNull<c_void>>,
    surface: NonNull<c_void>,
}

fn native_window(
    window: &(impl raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle),
) -> Result<NativeWindow, String> {
    let handle = raw_window_handle::HasWindowHandle::window_handle(window)
        .map_err(|error| format!("read native window handle: {error}"))?;
    match handle.as_raw() {
        RawWindowHandle::AppKit(handle) => Ok(NativeWindow {
            display: None,
            surface: handle.ns_view,
        }),
        RawWindowHandle::Wayland(handle) => {
            let display = raw_window_handle::HasDisplayHandle::display_handle(window)
                .map_err(|error| format!("read native display handle: {error}"))?;
            let RawDisplayHandle::Wayland(display) = display.as_raw() else {
                return Err(
                    "GPUI returned mismatched Wayland window and display handles".to_owned(),
                );
            };
            Ok(NativeWindow {
                display: Some(display.display),
                surface: handle.surface,
            })
        }
        _ => Err("libghostty native surfaces require macOS or Wayland".to_owned()),
    }
}

fn key_modifiers(
    mut value: Modifiers,
    implied_shift: bool,
    has_text: bool,
) -> (Modifiers, Modifiers) {
    if implied_shift {
        value.insert(Modifiers::SHIFT);
    }
    let consumed = if has_text && value.contains(Modifiers::SHIFT) {
        Modifiers::SHIFT
    } else {
        Modifiers::empty()
    };
    (value, consumed)
}

struct NativeKey {
    keycode: u32,
    unshifted_codepoint: u32,
    implied_shift: bool,
}

fn native_key(key: &str) -> Option<NativeKey> {
    let (key, implied_shift) = unshifted_key(key);
    let unshifted_codepoint = match key {
        "space" => u32::from(' '),
        _ => single_codepoint(key).map_or(0, u32::from),
    };
    Some(NativeKey {
        keycode: native_keycode(key)?,
        unshifted_codepoint,
        implied_shift,
    })
}

fn single_codepoint(value: &str) -> Option<char> {
    let mut chars = value.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

fn unshifted_key(key: &str) -> (&str, bool) {
    match key {
        "!" => ("1", true),
        "@" => ("2", true),
        "#" => ("3", true),
        "$" => ("4", true),
        "%" => ("5", true),
        "^" => ("6", true),
        "&" => ("7", true),
        "*" => ("8", true),
        "(" => ("9", true),
        ")" => ("0", true),
        "_" => ("-", true),
        "+" => ("=", true),
        "{" => ("[", true),
        "}" => ("]", true),
        "|" => ("\\", true),
        ":" => (";", true),
        "\"" => ("'", true),
        "<" => (",", true),
        ">" => (".", true),
        "?" => ("/", true),
        "~" => ("`", true),
        _ => (key, false),
    }
}

#[cfg(target_os = "macos")]
fn native_keycode(key: &str) -> Option<u32> {
    // Native values mirror Ghostty's pinned macOS keycode table. GPUI does
    // not expose NSEvent.keyCode, so keys it collapses (notably the keypad)
    // cannot be distinguished here.
    Some(match key {
        // ANSI printable keys, ordered by macOS virtual keycode.
        "a" => 0,
        "s" => 1,
        "d" => 2,
        "f" => 3,
        "h" => 4,
        "g" => 5,
        "z" => 6,
        "x" => 7,
        "c" => 8,
        "v" => 9,
        "b" => 11,
        "q" => 12,
        "w" => 13,
        "e" => 14,
        "r" => 15,
        "y" => 16,
        "t" => 17,
        "1" => 18,
        "2" => 19,
        "3" => 20,
        "4" => 21,
        "6" => 22,
        "5" => 23,
        "=" => 24,
        "9" => 25,
        "7" => 26,
        "-" => 27,
        "8" => 28,
        "0" => 29,
        "]" => 30,
        "o" => 31,
        "u" => 32,
        "[" => 33,
        "i" => 34,
        "p" => 35,
        "l" => 37,
        "j" => 38,
        "'" => 39,
        "k" => 40,
        ";" => 41,
        "\\" => 42,
        "," => 43,
        "/" => 44,
        "n" => 45,
        "m" => 46,
        "." => 47,
        "`" => 50,

        // Editing and navigation keys emitted by GPUI.
        "enter" | "return" => 36,
        "tab" => 48,
        "space" => 49,
        "backspace" => 51,
        "escape" => 53,
        "insert" => 114,
        "home" => 115,
        "pageup" | "page_up" | "page-up" => 116,
        "delete" => 117,
        "end" => 119,
        "pagedown" | "page_down" | "page-down" => 121,
        "left" => 123,
        "right" => 124,
        "down" => 125,
        "up" => 126,

        // Function keys available in Ghostty's macOS keycode table.
        "f1" => 122,
        "f2" => 120,
        "f3" => 99,
        "f4" => 118,
        "f5" => 96,
        "f6" => 97,
        "f7" => 98,
        "f8" => 100,
        "f9" => 101,
        "f10" => 109,
        "f11" => 103,
        "f12" => 111,
        "f13" => 105,
        "f14" => 107,
        "f15" => 113,
        "f16" => 106,
        "f17" => 64,
        "f18" => 79,
        "f19" => 80,
        "f20" => 90,
        _ => return None,
    })
}

#[cfg(target_os = "linux")]
fn native_keycode(key: &str) -> Option<u32> {
    // XKB keycodes used by Ghostty's Linux key table (evdev codes plus 8).
    Some(match key {
        "escape" => 9,
        "1" => 10,
        "2" => 11,
        "3" => 12,
        "4" => 13,
        "5" => 14,
        "6" => 15,
        "7" => 16,
        "8" => 17,
        "9" => 18,
        "0" => 19,
        "-" => 20,
        "=" => 21,
        "backspace" => 22,
        "tab" => 23,
        "q" => 24,
        "w" => 25,
        "e" => 26,
        "r" => 27,
        "t" => 28,
        "y" => 29,
        "u" => 30,
        "i" => 31,
        "o" => 32,
        "p" => 33,
        "[" => 34,
        "]" => 35,
        "enter" | "return" => 36,
        "a" => 38,
        "s" => 39,
        "d" => 40,
        "f" => 41,
        "g" => 42,
        "h" => 43,
        "j" => 44,
        "k" => 45,
        "l" => 46,
        ";" => 47,
        "'" => 48,
        "`" => 49,
        "\\" => 51,
        "z" => 52,
        "x" => 53,
        "c" => 54,
        "v" => 55,
        "b" => 56,
        "n" => 57,
        "m" => 58,
        "," => 59,
        "." => 60,
        "/" => 61,
        "space" => 65,
        "f1" => 67,
        "f2" => 68,
        "f3" => 69,
        "f4" => 70,
        "f5" => 71,
        "f6" => 72,
        "f7" => 73,
        "f8" => 74,
        "f9" => 75,
        "f10" => 76,
        "f11" => 95,
        "f12" => 96,
        "f13" => 191,
        "f14" => 192,
        "f15" => 193,
        "f16" => 194,
        "f17" => 195,
        "f18" => 196,
        "f19" => 197,
        "f20" => 198,
        "home" => 110,
        "up" => 111,
        "pageup" | "page_up" | "page-up" => 112,
        "left" => 113,
        "right" => 114,
        "end" => 115,
        "down" => 116,
        "pagedown" | "page_down" | "page-down" => 117,
        "insert" => 118,
        "delete" => 119,
        _ => return None,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn native_keycode(_: &str) -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_mapping_covers_neovim_and_missing_gpui_keys() {
        for key in [
            "h", "j", "k", "l", "escape", "insert", "home", "pageup", "delete", "end", "pagedown",
            "left", "right", "down", "up", "f1", "f20",
        ] {
            assert!(native_key(key).is_some(), "missing {key}");
        }
    }

    #[test]
    fn shifted_printable_keys_preserve_their_physical_key_and_consumed_shift() {
        for (shifted, unshifted) in [
            ("!", "1"),
            ("@", "2"),
            ("#", "3"),
            ("$", "4"),
            ("%", "5"),
            ("^", "6"),
            ("&", "7"),
            ("*", "8"),
            ("(", "9"),
            (")", "0"),
            ("_", "-"),
            ("+", "="),
            ("{", "["),
            ("}", "]"),
            ("|", "\\"),
            (":", ";"),
            ("\"", "'"),
            ("<", ","),
            (">", "."),
            ("?", "/"),
            ("~", "`"),
        ] {
            let key = native_key(shifted).expect("shifted key should map");
            let base = native_key(unshifted).expect("base key should map");
            let (active, consumed) = key_modifiers(Modifiers::empty(), key.implied_shift, true);
            assert_eq!(key.keycode, base.keycode);
            assert_eq!(key.unshifted_codepoint, base.unshifted_codepoint);
            assert_eq!(active, Modifiers::SHIFT);
            assert_eq!(consumed, Modifiers::SHIFT);
        }
    }

    #[test]
    fn named_keys_do_not_leak_their_names_as_unicode() {
        assert_eq!(
            native_key("space")
                .expect("space should map")
                .unshifted_codepoint,
            u32::from(' ')
        );
        for key in ["enter", "escape", "f1", "insert", "up"] {
            assert_eq!(
                native_key(key)
                    .expect("named key should map")
                    .unshifted_codepoint,
                0,
                "{key}"
            );
        }
    }

    #[test]
    fn shift_is_only_consumed_when_the_key_has_text() {
        let modifiers = Modifiers::SHIFT;
        let (active, consumed) = key_modifiers(modifiers, false, false);
        assert_eq!(active, Modifiers::SHIFT);
        assert_eq!(consumed, Modifiers::empty());
    }

    #[test]
    fn terminal_options_use_bare_defaults_by_default() {
        let options = TerminalOptions::new("sh", ".");
        assert_eq!(options.configuration, TerminalConfiguration::Default);
    }

    #[test]
    fn terminal_theme_serializes_to_ghostty_config() {
        let palette = std::array::from_fn(|index| {
            TerminalColor::new(index as u8, index as u8 + 1, index as u8 + 2)
        });
        let theme = TerminalTheme::new(
            TerminalColor::new(0x1d, 0x20, 0x21),
            TerminalColor::new(0xd5, 0xc4, 0xa1),
            palette,
        );

        let config = theme_config_contents(&theme);

        assert!(
            config.starts_with("background = #1d2021\nforeground = #d5c4a1\npalette = 0=#000102\n")
        );
        assert!(config.contains("palette = 15=#0f1011\n"));
        assert_eq!(config.matches("palette = ").count(), 16);
        assert!(config.ends_with("palette-generate = true\n"));
    }

    #[test]
    fn temporary_theme_config_is_removed_when_released() {
        let theme = TerminalTheme::new(
            TerminalColor::new(0, 0, 0),
            TerminalColor::new(255, 255, 255),
            [TerminalColor::new(0, 0, 0); 16],
        );
        let file = write_theme_config(&theme).expect("theme config should be writable");
        let path = file.path().to_owned();
        assert!(path.exists());

        drop(file);

        assert!(!path.exists());
    }
}
