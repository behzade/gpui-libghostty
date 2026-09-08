use std::path::PathBuf;

use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_neovim::{NvimEditor, NvimOptions};

fn main() {
    let cwd = std::env::current_dir().expect("read current directory");
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .map(|path| cwd.join(path))
        .unwrap_or_else(|| cwd.clone());
    let project = if path.is_dir() { path.clone() } else { cwd };

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
                    let mut editor = NvimEditor::spawn(NvimOptions::new(project, path), window, cx)
                        .expect("start embedded Neovim (install nvim or set GPUI_NVIM)");
                    editor.focus(window, cx);
                    editor
                })
            },
        )
        .expect("open Neovim window");
        cx.activate(true);
    });
}
