# Ghostty source

`ghostty.json` pins Ghostty 1.4.0 by commit and archive SHA-256.
`files.sha256` lists the source files needed by our macOS Metal and Linux
OpenGL builds, with their original upstream checksums. Files outside this
build closure (application assets, test fixtures, and documentation) are
not extracted. Every included file remains byte-for-byte upstream.

`ghostty/` is generated and ignored by Git. Populate it with:

```sh
cargo run --locked -p xtask -- prepare-ghostty
```

This verifies the archive, extracts the selected files, and checks all local
patches. It refuses to overwrite changed upstream files. To refresh a pin,
remove the generated `ghostty/` directory first; keep the pin, source inventory,
patches, and patched checksums in the same change.

The build script uses the bundled tree if present. Otherwise, Git/path builds
fetch the pinned archive into the source cache and extract there. They never
write into the dependency checkout. Source preparation uses Rust libraries;
consumers do not need Git, Python, curl, or a patch command.

For offline checkout builds, set `GHOSTTY_SOURCE_ARCHIVE` to the absolute path
of the pinned archive, or prepare `ghostty/` in advance. The supplied archive
must pass the same checksum checks. `GHOSTTY_SOURCE_CACHE_DIR` can select an
absolute cache directory; by default the build uses the existing native cache
root's sibling `source` directory. The maintainer tool uses `target/ghostty-source`.
Set `CARGO_NET_OFFLINE=true` to prohibit source downloads. Cargo's `--offline`
flag alone does not communicate that restriction to arbitrary build scripts.

Published crates include `ghostty/` through an explicit Cargo `include` list,
despite the Git ignore rule. Release CI prepares the source and checks the
actual `.crate` archive before publishing. Registry consumers therefore do
not download Ghostty source. Zig's existing dependency downloads and native
toolchain requirements are unchanged by this layout.

Cargo treats the included, ignored files as uncommitted, so packaging and
publishing use `--allow-dirty`. CI first checks that Git reports no tracked or
unignored changes, then verifies the bundled files against the committed
checksums. For a manual local archive, run the following after source preparation:

```sh
cargo package --locked --no-verify --allow-dirty -p gpui-libghostty
```

Local integration changes live in `../patches/ghostty/`; adapters live in
`../shim/` and `../src/`. Build output contains a patched copy. Never edit
`ghostty/` to change our integration.

Upstream: https://github.com/ghostty-org/ghostty

License: MIT; see `LICENSE` (also included in the generated source tree).
