# gpui-libghostty

Native Ghostty terminal and embedded Neovim components for GPUI.

## Demo

Embedded Neovim editing this project's README with completion:

https://github.com/user-attachments/assets/140e3552-1074-4994-b91d-d0966fe623c9

## Status
Project status is alpha, expect bugs and instability.

## Crates

- `gpui-libghostty` hosts a command in Ghostty's native Metal renderer on
  macOS or its OpenGL renderer in a native Wayland subsurface on Linux, and
  forwards GPUI keyboard, key-repeat, mouse, scroll, focus, resize, and
  visibility events.
- `gpui-neovim` starts Neovim with a private RPC socket and opens later files in
  the same editor instance.

The native renderer supports macOS and Wayland. Linux uses a caller-owned EGL
OpenGL 4.3 context and does not depend on Ghostty's GTK application runtime or
copy frames through CPU memory. Fractional scaling uses `wp_viewporter` when
available and falls back to an integer Wayland buffer scale. X11 is
intentionally unsupported. Ghostty is
pinned to commit
`9f0e1719dc918368367d368bfe300f59bb68b5a4`; the required, pruned source closure
is under `crates/gpui-ghostty/vendor/ghostty` so Cargo and Crane include it
with the package.

## Requirements

- macOS and Xcode command-line tools, or Wayland with EGL, libc++, and desktop OpenGL 4.3
- Zig 0.16
- Neovim for `gpui-neovim`

Set `ZIG` to select a non-default Zig executable. Set `GPUI_NVIM` or assign
`NvimOptions::executable` to select Neovim.

## Native build cache

The first build compiles Ghostty with Zig. Later builds reuse the native archive
across Cargo workspaces. The default cache is
`$XDG_CACHE_HOME/gpui-libghostty`, `$HOME/Library/Caches/gpui-libghostty` on
macOS, or `$HOME/.cache/gpui-libghostty` on other systems. If the shared cache
cannot be created, the build falls back to the current Cargo target directory.

Set `GHOSTTY_NATIVE_CACHE_DIR`, `GHOSTTY_ZIG_PACKAGE_CACHE_DIR`, or
`GHOSTTY_ZIG_GLOBAL_CACHE_DIR` to absolute paths to override each cache. The
native cache key includes the Ghostty source, target, Zig version, SDK, and
build options, so a changed input gets a new archive.

Set `GHOSTTY_ZIG_SYSTEM_PACKAGE_DIR` to make the build pass that path to
`zig build --system`. Zig will not download packages in this mode, so the
directory must contain every package required by the vendored Ghostty source.

## Terminal

```toml
[dependencies]
gpui-ghostty = { package = "gpui-libghostty", version = "0.1" }
```

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
