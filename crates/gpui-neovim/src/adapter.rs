/// Defines `NvimEditor` and `Terminal` against the caller's GPUI path.
///
/// Invoke once per module: `gpui_neovim::bind_gpui!(gpui);`.
/// This includes the terminal binding; do not invoke both macros in one module.
#[macro_export]
macro_rules! bind_gpui {
    ($gpui:path) => {
        pub use self::__gpui_neovim_adapter::{NvimEditor, Terminal};
        use $gpui as __gpui_neovim;
        mod __gpui_neovim_adapter {
            use super::__gpui_neovim as gpui;
            use gpui::{App, Context, Entity, IntoElement, Render, RenderImage, Task, Window};
            use std::{
                path::{Path, PathBuf},
                sync::Arc,
            };
            use $crate::NvimOptions;
            $crate::__private::ghostty::bind_gpui!(self::gpui);
            /// A Neovim process hosted inside a [`Terminal`].
            pub struct NvimEditor {
                session: $crate::NvimSession,
                terminal: Entity<Terminal>,
            }

            impl NvimEditor {
                pub fn spawn<T: 'static>(
                    options: NvimOptions,
                    window: &mut Window,
                    cx: &mut Context<T>,
                ) -> Result<Self, String> {
                    let (session, options) = $crate::NvimSession::new(options);
                    let terminal = Terminal::spawn(options, window, cx)?;
                    Ok(Self { session, terminal })
                }

                pub fn project(&self) -> &Path {
                    &self.session.project
                }

                pub fn path(&self) -> &Path {
                    &self.session.path
                }

                pub fn is_alive(&self, cx: &App) -> bool {
                    self.terminal.read(cx).is_alive()
                }

                pub fn focus<T>(&mut self, window: &mut Window, cx: &mut Context<T>) {
                    self.terminal
                        .update(cx, |terminal, cx| terminal.focus(window, cx));
                }

                pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
                    self.terminal
                        .update(cx, |terminal, _| terminal.set_visible(visible));
                }

                /// Captures Neovim's last completed native frame for temporary GPUI compositing.
                pub fn snapshot(
                    &mut self,
                    cx: &mut Context<Self>,
                ) -> Result<Arc<RenderImage>, String> {
                    self.terminal.update(cx, |terminal, _| terminal.snapshot())
                }

                /// Opens `path` in the existing Neovim server without blocking the UI thread.
                pub fn open_file(
                    &mut self,
                    path: PathBuf,
                    cx: &mut Context<Self>,
                ) -> Task<Result<(), String>> {
                    self.open_file_at_line(path, None, cx)
                }

                /// Opens `path`, places the cursor at `line`, and completes when Neovim accepts the request.
                pub fn open_file_at_line(
                    &mut self,
                    path: PathBuf,
                    line: Option<u64>,
                    cx: &mut Context<Self>,
                ) -> Task<Result<(), String>> {
                    if !self.terminal.read(cx).is_alive() {
                        return Task::ready(Err(
                            "the embedded Neovim process has exited".to_owned()
                        ));
                    }

                    let request = self.session.open_request(&path, line);
                    let request = cx.background_executor().spawn(async move { request.run() });

                    cx.spawn(async move |editor, cx| {
                        request.await?;
                        editor
                            .update(cx, |editor, _| editor.session.path = path)
                            .map_err(|_| "the embedded Neovim editor was dropped".to_owned())?;
                        Ok(())
                    })
                }
            }

            impl Render for NvimEditor {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    self.terminal.clone()
                }
            }
        }
    };
}
