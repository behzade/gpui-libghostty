# Vendored Ghostty

- Upstream: https://github.com/ghostty-org/ghostty
- Commit: `9f0e1719dc918368367d368bfe300f59bb68b5a4`
- License: MIT (`LICENSE`)

This vendor contains the source closure required to build Ghostty's internal
macOS surface library. Application, packaging, documentation, test, and
non-build assets were removed. Upstream-only test fonts and fixtures were also
removed from `src/font/embedded.zig` and `pkg/wuffs/src`.

Local patches:

- `build.zig` installs the macOS static `libghostty-internal.a` without building
  an XCFramework.
- `src/build/SharedDeps.zig` accepts `SDKROOT` for Darwin system headers. This
  avoids Nix's compiler wrapper hiding the selected Xcode SDK.
