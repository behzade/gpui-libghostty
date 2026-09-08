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

- Rust 1.95
- macOS and Xcode command-line tools, or Wayland with EGL, libc++ 21 or newer,
  libxml2, and desktop OpenGL 4.3
- Zig 0.16
- Neovim for `gpui-neovim`

The default Nix development shell provides the Rust tools, Zig, Neovim, and
the required Linux build and runtime libraries. macOS still requires Xcode
command-line tools because the native build uses `xcrun`.

Linux builds need the libc++ and libxml2 development packages as well as their
runtime libraries. Zig 0.16's C++ headers require libc++ 21 or newer; an older
runtime can fail to link with an undefined `std::__1::__hash_memory` symbol.
On Ubuntu 24.04, install `libc++-21-dev` and `libc++abi-21-dev` from
[LLVM's APT repository](https://apt.llvm.org/), plus `libxml2-dev` from Ubuntu.

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
gpui-ghostty = { package = "gpui-libghostty", version = "0.2" }
```

```rust,ignore
use gpui_ghostty::{Terminal, TerminalOptions};

let terminal = Terminal::spawn(
    TerminalOptions::new("bash", project_directory),
    window,
    cx,
)?;
```

`TerminalOptions::configuration` selects bare Ghostty defaults, the user's
Ghostty configuration, or application-owned colors:

```rust,ignore
use gpui_ghostty::{
    TerminalColor, TerminalConfiguration, TerminalOptions, TerminalTheme,
};

let mut options = TerminalOptions::new("bash", project_directory);
options.configuration = TerminalConfiguration::UserDefault;

let theme = TerminalTheme::new(
    TerminalColor::new(0x1d, 0x20, 0x21),
    TerminalColor::new(0xd5, 0xc4, 0xa1),
    ansi_palette,
);
options.configuration = TerminalConfiguration::Custom(theme);
// Or preserve user settings while replacing their colors:
options.configuration = TerminalConfiguration::UserDefaultWithOverride(theme);
```

`Default` is selected by `TerminalOptions::new` and reads no user files.
`UserDefault` loads Ghostty's default and recursively referenced configuration
files. `Custom` starts from bare defaults and applies the supplied colors.
`UserDefaultWithOverride` loads user configuration first, then applies the
supplied colors so they win. Custom colors generate the remaining indexed
colors from the supplied 16-color palette.

`Terminal::spawn` returns `Entity<Terminal>`, which can be rendered directly as
a GPUI child. `Terminal::snapshot` performs a one-shot GPU readback and returns
a `gpui::RenderImage`; render it at the terminal's logical bounds when temporarily
replacing the native surface beneath a normal GPUI overlay.

### Clipboard approval and focus

Protected clipboard operations are denied unless `TerminalOptions::clipboard_approval`
accepts them. This includes OSC 52 reads with Ghostty's default `clipboard-read = ask`,
unsafe pastes, and writes configured with `clipboard-write = ask`. Explicit Ghostty
`allow`/`deny` settings still apply; ordinary safe pastes do not need approval.

```rust,ignore
use std::sync::Arc;
use gpui_ghostty::ClipboardOperation;

// Example application policy: permit writes that Ghostty asks to confirm,
// but deny protected reads and unsafe pastes.
options.clipboard_approval = Some(Arc::new(|request| {
    request.operation == ClipboardOperation::Write
}));
```

The callback receives the operation and text. It is synchronous and must not
block, re-enter the terminal, or open a modal event loop. Missing or panicking
callbacks deny access. Denied reads return an empty response; denied unsafe
pastes insert nothing. Treat callback text as sensitive and avoid logging it.
`NvimOptions::clipboard_approval` exposes the same policy for embedded Neovim.

`Terminal::set_visible` does not steal keyboard focus, and hidden native surfaces
stay hidden across layout and resize. Native focus follows GPUI entity focus and
window activation. Use `Terminal::focus` to explicitly show and focus a terminal.

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

Call and await `NvimEditor::open_file` through the entity to reuse the running
Neovim instance. Remote requests run off the UI thread and return the error
reported by Neovim when they fail.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

`.github/workflows/ci.yml` runs these checks, including the native build, on
macOS and Linux for pull requests and main-branch pushes. Tagged releases must
pass the same matrix before publishing. These are build and unit-test checks,
not interactive AppKit/Wayland rendering tests.

## Versioning

GPUI is pinned to Zed commit `cc053a4a6fa2fd0e8793201ed9099466af1be0b1`.
Consumers using another GPUI source should patch that dependency consistently
so entity and event types remain identical.

## License

The workspace is MIT-licensed. Vendored Ghostty remains MIT-licensed; see
`crates/gpui-ghostty/vendor/ghostty/LICENSE` and
`crates/gpui-ghostty/vendor/ghostty/VENDOR.md`.
