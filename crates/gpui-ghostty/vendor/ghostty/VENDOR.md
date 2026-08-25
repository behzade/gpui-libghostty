# Vendored Ghostty

- Upstream: https://github.com/ghostty-org/ghostty
- Commit: `9f0e1719dc918368367d368bfe300f59bb68b5a4`
- License: MIT (`LICENSE`)

This vendor contains the source closure required to build Ghostty's internal
macOS Metal and embedded Linux OpenGL surface libraries. Application,
packaging, documentation, test, and non-build assets were removed. Upstream-only test fonts and fixtures were also
removed from `src/font/embedded.zig` and `pkg/wuffs/src`.

Local patches:

- `build.zig` installs only the internal static archive on macOS and Linux;
  macOS does so without building an XCFramework.
- `src/apprt/embedded.zig`, `src/renderer/OpenGL.zig`, and `include/ghostty.h`
  expose a caller-owned OpenGL context for the Wayland host adapter.
- `src/build/SharedDeps.zig` includes GLAD in OpenGL library artifacts and
  accepts `SDKROOT` for Darwin system headers. This avoids Nix's compiler
  wrapper hiding the selected Xcode SDK.
