# gpui-libghostty

Native Ghostty terminal component for GPUI.

## Demo

Neovim running inside `gpui-libghostty`, editing this project's README with completion:

https://github.com/user-attachments/assets/140e3552-1074-4994-b91d-d0966fe623c9

## Status
Project status is alpha, expect bugs and instability.

## Crate

Published on crates.io: [gpui-libghostty](https://crates.io/crates/gpui-libghostty). Add it to your project:

```sh
cargo add gpui-libghostty
```

Supports macOS and Linux with Wayland.

## Requirements

- Rust 1.95
- macOS and Xcode command-line tools, or Wayland with EGL, libc++ 22 or newer,
  libxml2, and desktop OpenGL 4.3
- Zig 0.17
- Neovim (optional, for the standalone Neovim example)

The default Nix development shell provides the Rust tools, Zig, Neovim, and
the required Linux build and runtime libraries. macOS still requires Xcode
command-line tools because the native build uses `xcrun`.

On Ubuntu 24.04, install `libc++-22-dev` and `libc++abi-22-dev` from
[LLVM's APT repository](https://apt.llvm.org/), plus `libxml2-dev` from Ubuntu.

Set `ZIG` to select a non-default Zig executable.

## Terminal

```toml
[dependencies]
gpui-libghostty = "0.4"
```

```rust,ignore
use gpui_libghostty::TerminalOptions;

gpui_libghostty::bind_gpui!(gpui);

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

### Live theming

`Terminal::update_theme` reapplies colors to a running terminal without
restarting its process, which lets an application repaint its terminals when the
user switches themes:

```rust,ignore
use gpui_libghostty::{TerminalColor, TerminalTheme};

let theme = TerminalTheme::new(
    TerminalColor::new(0x1d, 0x20, 0x21),
    TerminalColor::new(0xd5, 0xc4, 0xa1),
    palette,
);
terminal.update(cx, |terminal, _| {
    let _ = terminal.update_theme(theme);
});
```

### Clipboard approval

The adapter supports plain text clipboard data. Binary formats and MIME-list
requests are not supported.

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

## Example

Run the standalone Neovim example from this repository:

```sh
cargo run --bin neovim -- README.md
```

The optional argument is a file or directory. The example uses your Neovim
configuration; install `nvim` on `PATH` or set `GPUI_NVIM` to its executable.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## GPUI compatibility

This library does not depend on GPUI. `bind_gpui!` uses your application's GPUI
crate or module path, including renamed packages such as `gpui-pre`.

Invoke `gpui_libghostty::bind_gpui!(gpui)` once at module scope, outside
`fn main()`, and share the generated `Terminal` type across your app.

Checks cover the workspace's Zed revision and `gpui-pre` 0.3.4. GPUI API changes
may require an adapter update; arbitrary revisions are not guaranteed.

## Ghostty source and local changes

Ghostty's source is pinned by commit and checksum. Git checkouts fetch it
automatically during the native build; published crates already include it.
Our changes to Ghostty live in `crates/gpui-ghostty/patches/ghostty/` and apply
to a build copy, leaving upstream files unchanged. Rust APIs and consumer
build commands are unchanged.

Maintainers can prepare the ignored upstream tree before packaging with:

```sh
cargo run --locked -p xtask -- prepare-ghostty
```

See [source preparation](crates/gpui-ghostty/upstream/README.md) for offline
builds and release packaging, and [our patch list](crates/gpui-ghostty/patches/ghostty/README.md)
for the integration changes.

## License

The workspace and Ghostty are MIT-licensed; see
`crates/gpui-ghostty/upstream/LICENSE` for Ghostty's license.
