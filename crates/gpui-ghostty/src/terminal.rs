use std::{
    ffi::{CString, c_void},
    path::PathBuf,
    ptr::NonNull,
    time::Duration,
};

use gpui::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, KeyUpEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _,
    Pixels, Render, ScrollDelta, ScrollWheelEvent, Styled as _, Task, Window, canvas, div,
};
use raw_window_handle::RawWindowHandle;

use crate::native::{KeyAction, Modifiers, MouseButton, MouseState, NativeSurface};

const TICK_INTERVAL: Duration = Duration::from_millis(8);

/// Configuration for a terminal process rendered by libghostty.
pub struct TerminalOptions {
    pub command: String,
    pub working_directory: PathBuf,
    pub focus_on_spawn: bool,
}

impl TerminalOptions {
    pub fn new(command: impl Into<String>, working_directory: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            working_directory: working_directory.into(),
            focus_on_spawn: true,
        }
    }
}

/// A GPUI entity backed by Ghostty's native macOS Metal surface.
pub struct Terminal {
    surface: NativeSurface,
    focus: FocusHandle,
    bounds: Bounds<Pixels>,
    tick_task: Option<Task<()>>,
}

impl Terminal {
    /// Spawns the configured command and attaches its native surface to `window`.
    pub fn spawn<T: 'static>(
        options: TerminalOptions,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Result<Entity<Self>, String> {
        let working_directory =
            CString::new(options.working_directory.to_string_lossy().as_bytes()).map_err(|_| {
                format!(
                    "terminal working directory contains a NUL byte: {}",
                    options.working_directory.display()
                )
            })?;
        let command = CString::new(options.command)
            .map_err(|_| "terminal command contains a NUL byte".to_owned())?;
        let parent_view = appkit_view(window)?;
        let surface = NativeSurface::new(parent_view, &working_directory, &command)
            .map_err(|error| format!("initialize libghostty: {error}"))?;
        let focus = cx.focus_handle();
        if options.focus_on_spawn {
            focus.focus(window, cx);
        }
        Ok(cx.new(|_| Self {
            surface,
            focus,
            bounds: Bounds::default(),
            tick_task: None,
        }))
    }

    pub fn is_alive(&self) -> bool {
        self.surface.is_alive()
    }

    pub fn focus<T>(&mut self, window: &mut Window, cx: &mut Context<T>) {
        self.surface.set_visible(true);
        self.surface.set_focus(true);
        self.focus.focus(window, cx);
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.surface.set_visible(visible);
        self.surface.set_focus(visible);
    }

    fn start_ticking(&mut self, cx: &mut Context<Self>) {
        if self.tick_task.is_some() {
            return;
        }
        self.surface.tick();
        let terminal = cx.entity().downgrade();
        self.tick_task = Some(cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor().timer(TICK_INTERVAL).await;
                let updated = terminal.update(cx, |terminal, cx| {
                    if terminal.surface.needs_tick() {
                        terminal.surface.tick();
                        cx.notify();
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        }));
    }

    fn update_frame(&mut self, bounds: Bounds<Pixels>) {
        self.bounds = bounds;
        self.surface.set_frame(
            f64::from(f32::from(bounds.origin.x)),
            f64::from(f32::from(bounds.origin.y)),
            f64::from(f32::from(bounds.size.width)),
            f64::from(f32::from(bounds.size.height)),
        );
        self.surface.set_visible(true);
    }

    fn key_down(&mut self, event: &KeyDownEvent) {
        self.send_key(
            if event.is_held {
                KeyAction::Repeat
            } else {
                KeyAction::Press
            },
            &event.keystroke,
        );
    }

    fn key_up(&mut self, event: &KeyUpEvent) {
        self.send_key(KeyAction::Release, &event.keystroke);
    }

    fn send_key(&mut self, action: KeyAction, keystroke: &gpui::Keystroke) {
        let Some(keycode) = mac_keycode(&keystroke.key) else {
            if matches!(action, KeyAction::Press | KeyAction::Repeat)
                && !keystroke.modifiers.control
                && !keystroke.modifiers.alt
                && !keystroke.modifiers.platform
                && let Some(text) = keystroke.key_char.as_deref()
                && let Ok(text) = CString::new(text)
            {
                self.surface.text(&text);
            }
            return;
        };
        let text = keystroke
            .key_char
            .as_deref()
            .and_then(|text| CString::new(text).ok());
        let unshifted = keystroke.key.chars().next().map_or(0, u32::from);
        let _ = self.surface.key(
            action,
            modifiers(keystroke.modifiers),
            keycode,
            text.as_deref(),
            unshifted,
        );
    }

    fn mouse_position(&mut self, position: gpui::Point<Pixels>, modifiers: gpui::Modifiers) {
        let x = f64::from(f32::from(position.x - self.bounds.origin.x));
        let y = f64::from(f32::from(position.y - self.bounds.origin.y));
        self.surface.mouse_position(x, y, modifiers.into());
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.surface.set_focus(true);
        self.mouse_position(event.position, event.modifiers);
        self.surface.mouse_button(
            MouseState::Press,
            event.button.into(),
            event.modifiers.into(),
        );
    }

    fn mouse_up(&mut self, event: &MouseUpEvent) {
        self.mouse_position(event.position, event.modifiers);
        self.surface.mouse_button(
            MouseState::Release,
            event.button.into(),
            event.modifiers.into(),
        );
    }

    fn scroll(&mut self, event: &ScrollWheelEvent) {
        self.mouse_position(event.position, event.modifiers);
        let (x, y, precision) = match event.delta {
            ScrollDelta::Pixels(delta) => (
                f64::from(f32::from(delta.x)),
                f64::from(f32::from(delta.y)),
                true,
            ),
            ScrollDelta::Lines(delta) => (f64::from(delta.x), f64::from(delta.y), false),
        };
        self.surface.mouse_scroll(x, y, precision);
    }
}

impl Render for Terminal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.start_ticking(cx);
        let terminal = cx.entity().downgrade();
        div()
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let _ = terminal.update(cx, |terminal, _| terminal.update_frame(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .on_key_down(cx.listener(|terminal, event, _, _| terminal.key_down(event)))
            .on_key_up(cx.listener(|terminal, event, _, _| terminal.key_up(event)))
            .on_mouse_move(cx.listener(|terminal, event: &MouseMoveEvent, _, _| {
                terminal.mouse_position(event.position, event.modifiers);
            }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|terminal, event, window, cx| terminal.mouse_down(event, window, cx)),
            )
            .on_mouse_down(
                gpui::MouseButton::Middle,
                cx.listener(|terminal, event, window, cx| terminal.mouse_down(event, window, cx)),
            )
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(|terminal, event, window, cx| terminal.mouse_down(event, window, cx)),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|terminal, event, _, _| terminal.mouse_up(event)),
            )
            .on_mouse_up(
                gpui::MouseButton::Middle,
                cx.listener(|terminal, event, _, _| terminal.mouse_up(event)),
            )
            .on_mouse_up(
                gpui::MouseButton::Right,
                cx.listener(|terminal, event, _, _| terminal.mouse_up(event)),
            )
            .on_scroll_wheel(cx.listener(|terminal, event, _, _| terminal.scroll(event)))
    }
}

impl From<gpui::Modifiers> for Modifiers {
    fn from(value: gpui::Modifiers) -> Self {
        modifiers(value)
    }
}

impl From<gpui::MouseButton> for MouseButton {
    fn from(value: gpui::MouseButton) -> Self {
        match value {
            gpui::MouseButton::Left => Self::Left,
            gpui::MouseButton::Right => Self::Right,
            gpui::MouseButton::Middle => Self::Middle,
            gpui::MouseButton::Navigate(_) => Self::Unknown,
        }
    }
}

fn appkit_view(window: &Window) -> Result<NonNull<c_void>, String> {
    let handle = raw_window_handle::HasWindowHandle::window_handle(window)
        .map_err(|error| format!("read native window handle: {error}"))?;
    match handle.as_raw() {
        RawWindowHandle::AppKit(handle) => Ok(handle.ns_view),
        _ => Err("libghostty native surfaces are currently available only on macOS".to_owned()),
    }
}

fn modifiers(value: gpui::Modifiers) -> Modifiers {
    let mut result = Modifiers::empty();
    if value.shift {
        result.insert(Modifiers::SHIFT);
    }
    if value.control {
        result.insert(Modifiers::CONTROL);
    }
    if value.alt {
        result.insert(Modifiers::ALT);
    }
    if value.platform {
        result.insert(Modifiers::SUPER);
    }
    result
}

fn mac_keycode(key: &str) -> Option<u32> {
    Some(match key {
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
        "enter" | "return" => 36,
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
        "tab" => 48,
        "space" => 49,
        "`" => 50,
        "backspace" => 51,
        "escape" => 53,
        "f17" => 64,
        "f18" => 79,
        "f19" => 80,
        "f20" => 90,
        "f5" => 96,
        "f6" => 97,
        "f7" => 98,
        "f3" => 99,
        "f8" => 100,
        "f9" => 101,
        "f11" => 103,
        "f13" => 105,
        "f16" => 106,
        "f14" => 107,
        "f10" => 109,
        "f12" => 111,
        "f15" => 113,
        "home" => 115,
        "pageup" | "page_up" | "page-up" => 116,
        "delete" => 117,
        "f4" => 118,
        "end" => 119,
        "f2" => 120,
        "pagedown" | "page_down" | "page-down" => 121,
        "left" => 123,
        "right" => 124,
        "down" => 125,
        "up" => 126,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycode_mapping_covers_terminal_navigation_and_repeat_keys() {
        for key in ["j", "k", "up", "down", "pageup", "pagedown", "escape"] {
            assert!(mac_keycode(key).is_some(), "missing keycode for {key}");
        }
    }
}
