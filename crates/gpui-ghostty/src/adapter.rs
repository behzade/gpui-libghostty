/// Defines `Terminal` using the caller's GPUI crate or module path.
///
/// Invoke once per module: `gpui_libghostty::bind_gpui!(gpui);`.
/// The selected GPUI must implement the API described in the README.
/// Re-export the generated type to share it across your application.
#[macro_export]
macro_rules! bind_gpui {
    ($gpui:path) => {
        pub use self::__gpui_ghostty_adapter::Terminal;
        use $gpui as __gpui_ghostty;
        mod __gpui_ghostty_adapter {
            use super::__gpui_ghostty as gpui;
            use gpui::{
                AppContext as _, Bounds, ClipboardItem, Context, Entity, FocusHandle,
                InteractiveElement as _, IntoElement, KeyDownEvent, KeyUpEvent, MouseDownEvent,
                MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Render, RenderImage,
                ScrollDelta, ScrollWheelEvent, Styled as _, Subscription, Task, Window, canvas,
                div,
            };
            use std::sync::Arc;

            use $crate::__private::{KeyAction, Modifiers, MouseButton, MouseState, NativeSurface};
            use $crate::TerminalOptions;
            /// A GPUI entity backed by Ghostty's native Metal or Wayland/OpenGL surface.
            pub struct Terminal {
                surface: NativeSurface,
                focus: FocusHandle,
                bounds: Bounds<Pixels>,
                tick_task: Option<Task<()>>,
                visible: bool,
                window_focused: bool,
                _subscriptions: Vec<Subscription>,
            }

            impl Terminal {
                /// Spawns the configured command and attaches its native surface to `window`.
                pub fn spawn<T: 'static>(
                    options: TerminalOptions,
                    window: &mut Window,
                    cx: &mut Context<T>,
                ) -> Result<Entity<Self>, String> {
                    let focus_on_spawn = options.focus_on_spawn;
                    // SAFETY: GPUI owns the parent window; this entity services and drops
                    // its native child on the window's UI thread, as in the native adapter.
                    let surface = unsafe {
                        $crate::__private::spawn_surface(
                            options,
                            window,
                            f64::from(window.scale_factor()),
                        )?
                    };
                    let focus = cx.focus_handle();
                    if focus_on_spawn {
                        focus.focus(window, cx);
                    }
                    Ok(cx.new(|cx| {
                        let subscriptions = vec![
                            cx.on_focus(&focus, window, |terminal: &mut Self, window, _| {
                                terminal.sync_focus(window)
                            }),
                            cx.on_blur(&focus, window, |terminal: &mut Self, window, _| {
                                terminal.sync_focus(window)
                            }),
                            cx.observe_window_activation(
                                window,
                                |terminal: &mut Self, window, _| terminal.sync_focus(window),
                            ),
                        ];
                        let mut terminal = Self {
                            surface,
                            focus,
                            bounds: Bounds::default(),
                            tick_task: None,
                            visible: true,
                            window_focused: false,
                            _subscriptions: subscriptions,
                        };
                        terminal.sync_focus(window);
                        terminal
                    }))
                }

                pub fn is_alive(&self) -> bool {
                    self.surface.is_alive()
                }

                pub fn focus<T>(&mut self, window: &mut Window, cx: &mut Context<T>) {
                    self.set_visible(true);
                    self.focus.focus(window, cx);
                    self.sync_focus(window);
                }

                /// Shows or hides the native child without changing GPUI keyboard focus.
                /// Hidden surfaces remain hidden across layout, resize, and scale changes.
                pub fn set_visible(&mut self, visible: bool) {
                    self.visible = visible;
                    self.surface.set_visible(visible);
                    self.surface.set_focus(self.visible && self.window_focused);
                }

                fn sync_focus(&mut self, window: &Window) {
                    self.window_focused =
                        self.focus.is_focused(window) && window.is_window_active();
                    self.surface.set_focus(self.visible && self.window_focused);
                }

                /// Captures the last completed native frame for temporary GPUI compositing.
                ///
                /// This performs a synchronous GPU readback and should only be used for
                /// infrequent transitions such as presenting a modal over the terminal.
                /// Render the image at the terminal's logical bounds because its pixels use
                /// the native surface's display scale.
                pub fn snapshot(&mut self) -> Result<Arc<RenderImage>, String> {
                    Ok(Arc::new(RenderImage::new([self
                        .surface
                        .snapshot_frame()?])))
                }

                fn start_ticking(&mut self, cx: &mut Context<Self>) {
                    if self.tick_task.is_some() {
                        return;
                    }
                    self.surface.tick();
                    self.service_clipboard(cx);
                    let wakeup = self.surface.wakeup();
                    let terminal = cx.entity().downgrade();
                    self.tick_task = Some(cx.spawn(async move |_, cx| {
                        loop {
                            wakeup.wait().await;
                            let updated = terminal.update(cx, |terminal, cx| {
                                // Ghostty draws its native child during the tick; GPUI has no
                                // terminal pixels to repaint for this wakeup.
                                terminal.surface.tick();
                                terminal.service_clipboard(cx);
                            });
                            if updated.is_err() {
                                break;
                            }
                        }
                    }));
                }

                fn service_clipboard(&mut self, cx: &mut Context<Self>) {
                    self.surface.service_clipboard_read(|selection| {
                        let item = if selection {
                            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                            {
                                cx.read_from_primary()
                            }
                            #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
                            {
                                cx.read_from_clipboard()
                            }
                        } else {
                            cx.read_from_clipboard()
                        };
                        item.and_then(|item| item.text()).unwrap_or_default()
                    });

                    while let Some(write) = self.surface.take_clipboard_write() {
                        let item = ClipboardItem::new_string(write.text);
                        if write.selection {
                            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                            cx.write_to_primary(item);
                            #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
                            cx.write_to_clipboard(item);
                        } else {
                            cx.write_to_clipboard(item);
                        }
                    }
                }

                fn update_frame(&mut self, bounds: Bounds<Pixels>, scale_factor: f64) {
                    self.bounds = bounds;
                    self.surface.set_frame(
                        f64::from(f32::from(bounds.origin.x)),
                        f64::from(f32::from(bounds.origin.y)),
                        f64::from(f32::from(bounds.size.width)),
                        f64::from(f32::from(bounds.size.height)),
                        scale_factor,
                    );
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
                    self.surface.send_key(
                        action,
                        &keystroke.key,
                        keystroke.key_char.as_deref(),
                        modifiers(keystroke.modifiers),
                    );
                }

                fn mouse_position(
                    &mut self,
                    position: gpui::Point<Pixels>,
                    input_modifiers: gpui::Modifiers,
                ) {
                    let x = f64::from(f32::from(position.x - self.bounds.origin.x));
                    let y = f64::from(f32::from(position.y - self.bounds.origin.y));
                    self.surface
                        .mouse_position(x, y, modifiers(input_modifiers));
                }

                fn mouse_down(
                    &mut self,
                    event: &MouseDownEvent,
                    window: &mut Window,
                    cx: &mut Context<Self>,
                ) {
                    self.focus.focus(window, cx);
                    self.sync_focus(window);
                    self.mouse_position(event.position, event.modifiers);
                    self.surface.mouse_button(
                        MouseState::Press,
                        mouse_button(event.button),
                        modifiers(event.modifiers),
                    );
                }

                fn mouse_up(&mut self, event: &MouseUpEvent) {
                    self.mouse_position(event.position, event.modifiers);
                    self.surface.mouse_button(
                        MouseState::Release,
                        mouse_button(event.button),
                        modifiers(event.modifiers),
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
                        ScrollDelta::Lines(delta) => {
                            (f64::from(delta.x), f64::from(delta.y), false)
                        }
                    };
                    self.surface.mouse_scroll(x, y, precision);
                }
            }

            impl Render for Terminal {
                fn render(
                    &mut self,
                    window: &mut Window,
                    cx: &mut Context<Self>,
                ) -> impl IntoElement {
                    self.sync_focus(window);
                    self.start_ticking(cx);
                    let terminal = cx.entity().downgrade();
                    let mut element = div()
                        .key_context("Terminal")
                        .track_focus(&self.focus)
                        .size_full()
                        .min_h_0()
                        .child(
                            canvas(
                                move |bounds, window, cx| {
                                    let scale_factor = f64::from(window.scale_factor());
                                    let _ = terminal.update(cx, |terminal, _| {
                                        terminal.update_frame(bounds, scale_factor);
                                    });
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
                        }));
                    for button in [
                        gpui::MouseButton::Left,
                        gpui::MouseButton::Middle,
                        gpui::MouseButton::Right,
                    ] {
                        element = element
                            .on_mouse_down(
                                button,
                                cx.listener(|terminal, event, window, cx| {
                                    terminal.mouse_down(event, window, cx)
                                }),
                            )
                            .on_mouse_up(
                                button,
                                cx.listener(|terminal, event, _, _| terminal.mouse_up(event)),
                            );
                    }
                    element.on_scroll_wheel(
                        cx.listener(|terminal, event, _, _| terminal.scroll(event)),
                    )
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

            fn mouse_button(value: gpui::MouseButton) -> MouseButton {
                match value {
                    gpui::MouseButton::Left => MouseButton::Left,
                    gpui::MouseButton::Right => MouseButton::Right,
                    gpui::MouseButton::Middle => MouseButton::Middle,
                    _ => MouseButton::Unknown,
                }
            }
        }
    };
}
