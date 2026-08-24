# gpui-ghostty

Native Ghostty terminal and embedded Neovim components for GPUI.

## Crates

- `gpui-ghostty` hosts a command in Ghostty's native Metal renderer and forwards
  GPUI keyboard, key-repeat, mouse, scroll, focus, resize, and visibility events.
- `gpui-neovim` starts Neovim with a private RPC socket and opens later files in
  the same editor instance.

The native renderer currently supports macOS. Ghostty's embedded platform API
exposes macOS and iOS native views; Linux uses Ghostty's GTK application runtime
and needs a separate GTK/OpenGL host adapter. Ghostty is pinned to commit
`9f0e1719dc918368367d368bfe300f59bb68b5a4`; the required, pruned source closure
is under `crates/gpui-ghostty/vendor/ghostty` so Cargo and Crane include it
with the package.

## Requirements

- macOS and Xcode command-line tools
- Zig 0.16
- Neovim for `gpui-neovim`

Set `ZIG` to select a non-default Zig executable. Set `GPUI_NVIM` or assign
`NvimOptions::executable` to select Neovim.

## Terminal

```rust,ignore
use gpui_ghostty::{Terminal, TerminalOptions};

let terminal = Terminal::spawn(
    TerminalOptions::new("bash", project_directory),
    window,
    cx,
)?;
```

`Terminal::spawn` returns `Entity<Terminal>`, which can be rendered directly as
a GPUI child.

## Neovim

```rust,ignore
use gpui_neovim::{NvimEditor, NvimOptions};

let editor = NvimEditor::spawn(
    NvimOptions::new(project_directory, initial_file),
    window,
    cx,
)?;
let editor = cx.new(|_| editor);
```

Call `NvimEditor::open_file` through the entity to reuse the running Neovim
instance.

## Versioning

GPUI is pinned to Zed commit `cc053a4a6fa2fd0e8793201ed9099466af1be0b1`.
Consumers using another GPUI source should patch that dependency consistently
so entity and event types remain identical.

## License

The workspace is MIT-licensed. Vendored Ghostty remains MIT-licensed; see
`crates/gpui-ghostty/vendor/ghostty/LICENSE` and
`crates/gpui-ghostty/vendor/ghostty/VENDOR.md`.
