use std::path::{Path, PathBuf};

use gpui::{
    App, AppContext, Bounds, Context, Entity, IntoElement, Render, Window, WindowBounds,
    WindowOptions, px, size,
};
use gpui_libghostty::TerminalOptions;

gpui_libghostty::bind_gpui!(gpui);

struct Root(Entity<Terminal>);

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.clone()
    }
}

fn main() {
    let cwd = std::env::current_dir().expect("read current directory");
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .map(|path| cwd.join(path))
        .unwrap_or_else(|| cwd.clone());
    let project = if path.is_dir() { path.clone() } else { cwd };
    let executable = std::env::var_os("GPUI_NVIM")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("nvim"));
    let command = format!("{} -- {}", shell_quote(&executable), shell_quote(&path));

    gpui_platform::application().run(move |cx: &mut App| {
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1000.0), px(700.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                cx.new(|cx| {
                    let mut options = TerminalOptions::new(command, project);
                    options.quiet_login = true;
                    let terminal = Terminal::spawn(options, window, cx)
                        .expect("start embedded Neovim (install nvim or set GPUI_NVIM)");
                    terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
                    Root(terminal)
                })
            },
        )
        .expect("open Neovim window");
        cx.activate(true);
    });
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
