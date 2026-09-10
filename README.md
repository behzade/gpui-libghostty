# gpui-libghostty

Native Ghostty terminal and embedded Neovim components for GPUI.

## Demo

Embedded Neovim editing this project's README with completion:

https://github.com/user-attachments/assets/140e3552-1074-4994-b91d-d0966fe623c9

## Status
Project status is alpha, expect bugs and instability.

## Crates

Published on crates.io: [gpui-libghostty](https://crates.io/crates/gpui-libghostty)
and [gpui-neovim](https://crates.io/crates/gpui-neovim). Add either to your project:

```sh
cargo add gpui-libghostty
cargo add gpui-neovim
```

- `gpui-libghostty` embeds a native Ghostty terminal in GPUI.
- `gpui-neovim` embeds Neovim in GPUI.

Supports macOS and Linux with Wayland.

## Requirements

- Rust 1.95
- macOS and Xcode command-line tools, or Wayland with EGL, libc++ 21 or newer,
  libxml2, and desktop OpenGL 4.3
- Zig 0.16
- Neovim for `gpui-neovim`

The default Nix development shell provides the Rust tools, Zig, Neovim, and
the required Linux build and runtime libraries. macOS still requires Xcode
command-line tools because the native build uses `xcrun`.

On Ubuntu 24.04, install `libc++-21-dev` and `libc++abi-21-dev` from
[LLVM's APT repository](https://apt.llvm.org/), plus `libxml2-dev` from Ubuntu.

Set `ZIG` to select a non-default Zig executable. Set `GPUI_NVIM` or assign
`NvimOptions::executable` to select Neovim.

## Terminal

```toml
[dependencies]
gpui-libghostty = "0.2"
```

```rust,ignore
use gpui_libghostty::{Terminal, TerminalOptions};

let terminal = Terminal::spawn(
    TerminalOptions::new("bash", project_directory),
    window,
    cx,
)?;
```

`Terminal::spawn` returns an `Entity<Terminal>` you can render as a GPUI child.
To load the user's Ghostty configuration:

```rust,ignore
use gpui_libghostty::TerminalConfiguration;

let mut options = TerminalOptions::new("bash", project_directory);
options.configuration = TerminalConfiguration::UserDefault;
```

### Clipboard approval

Clipboard operations requiring approval are denied unless `clipboard_approval`
accepts them. Explicit Ghostty `allow`/`deny` settings still apply.

```rust,ignore
use std::sync::Arc;
use gpui_libghostty::ClipboardOperation;

// Example application policy: permit writes that Ghostty asks to confirm,
// but deny protected reads and unsafe pastes.
options.clipboard_approval = Some(Arc::new(|request| {
    request.operation == ClipboardOperation::Write
}));
```

`NvimOptions::clipboard_approval` exposes the same policy for embedded Neovim.

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
Neovim instance.

Run the standalone example from this repository:

```sh
cargo run -p gpui-neovim --example neovim -- README.md
```

The optional argument is a file or directory. The example uses your Neovim
configuration; install `nvim` on `PATH` or set `GPUI_NVIM` to its executable.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## Versioning

GPUI is pinned to Zed commit `cc053a4a6fa2fd0e8793201ed9099466af1be0b1`.
Consumers using another GPUI source should patch that dependency consistently
so entity and event types remain identical.

## License

The workspace is MIT-licensed. Vendored Ghostty remains MIT-licensed; see
`crates/gpui-ghostty/vendor/ghostty/LICENSE` and
`crates/gpui-ghostty/vendor/ghostty/VENDOR.md`.
