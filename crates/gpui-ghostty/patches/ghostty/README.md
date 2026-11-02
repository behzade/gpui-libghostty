# Our Ghostty patches

Each `.patch` targets the upstream path with the same name, without the
`.patch` suffix. `series` lists the application order. `files.sha256` records
the expected result for patched files; unpatched files retain the checksums
in `../../upstream/files.sha256`. A failed patch or checksum stops the build.

These patches preserve the integration previously maintained inside our
vendored Ghostty tree:

- `build.zig`: install the internal static archive on macOS and Linux,
  without requiring a macOS XCFramework build.
- `include/ghostty.h`, `src/apprt/embedded.zig`: expose the host-owned OpenGL
  context and use Wayland selection for primary clipboard requests.
- `src/renderer/{OpenGL,Thread,generic}.zig` and
  `src/renderer/opengl/{Device,Frame}.zig`: draw and free OpenGL resources on
  the application thread with the host context current; present to the host
  framebuffer instead of GTK's EGL/DMABUF path. Hidden surfaces retain their
  GPU resources until teardown.
- `src/build/SharedDeps.zig`: include GLAD for embedded OpenGL; use runtime
  Metal shader compilation together with `src/renderer/metal/shaders.zig`.
- `pkg/apple-sdk/build.zig`: honor `SDKROOT` for Darwin headers.
- `src/renderer/Metal.zig`: reject the OpenGL platform tag.
- `src/font/{embedded,SharedGridSet}.zig`: omit bundled fallback/test fonts;
  use system emoji fonts.
- `src/termio/Exec.zig`: consume `GHOSTTY_QUIET_LOGIN` per surface, preserve
  `.hushlogin`, and remove our marker from child environments.

To change a patch, work from a copy of the pristine source, apply the series,
make the change, and regenerate that file's unified diff against pristine
source. Update its resulting SHA-256 in `files.sha256`, then run:

```sh
cargo run --locked -p xtask -- check-ghostty
cargo test --locked -p xtask
```

`check-ghostty` prints the verified patched tree's path. Native builds and the
C shim tests use the headers from this patched tree. Source preparation never
patches the generated upstream tree in place.

The C/Objective-C clipboard and surface adapters are our code in `../../shim/`.
Changes there do not need an upstream patch. Upstream bugs in unchanged files
remain attributable to the pinned Ghostty commit.
