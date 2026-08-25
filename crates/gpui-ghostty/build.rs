use std::{
    env,
    ffi::OsStr,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    process::Command,
};

const NATIVE_CACHE_VERSION: &str = "1";
const GHOSTTY_BUILD_OPTIONS: &[&str] = &[
    "-Dapp-runtime=none",
    "-Demit-xcframework=false",
    "-Demit-macos-app=false",
    "-Demit-docs=false",
    "-Demit-terminfo=false",
    "-Demit-bench=false",
    "-Demit-webdata=false",
    "-Di18n=false",
    "-Dsentry=false",
    "-Doptimize=ReleaseFast",
];

fn main() {
    println!("cargo:rerun-if-changed=shim/ghostty_surface.m");
    println!("cargo:rerun-if-changed=vendor/ghostty");
    println!("cargo:rerun-if-env-changed=GHOSTTY_NATIVE_CACHE_DIR");
    println!("cargo:rerun-if-env-changed=GHOSTTY_ZIG_PACKAGE_CACHE_DIR");
    println!("cargo:rerun-if-env-changed=PATH");
    println!("cargo:rerun-if-env-changed=ZIG");

    if env::var_os("CARGO_CFG_TARGET_OS").as_deref() != Some(OsStr::new("macos")) {
        return;
    }

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let source = manifest.join("vendor/ghostty");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    let tools = NativeTools::detect();
    let fingerprint = native_fingerprint(&source, &tools);
    let cache_root = native_cache_root(&out_dir);
    let prefix = cache_root.join(&fingerprint);
    let library = prefix.join("lib/libghostty-internal.a");
    {
        std::fs::create_dir_all(&cache_root).expect("create shared Ghostty native cache");
        let cache_lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(cache_root.join(format!("{fingerprint}.lock")))
            .expect("open Ghostty native cache lock");
        File::lock(&cache_lock).expect("lock Ghostty native cache");

        if !library.exists() {
            if prefix.exists() {
                std::fs::remove_dir_all(&prefix).expect("remove incomplete Ghostty native build");
            }
            build_ghostty(&source, &out_dir, &prefix, &fingerprint, &tools);
        }
    }

    compile_shim(&manifest, &source, &out_dir, &tools);

    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=ghostty-internal");
    println!("cargo:rustc-link-lib=c++");
    println!("cargo:rustc-link-lib=objc");
    for framework in [
        "AppKit",
        "Carbon",
        "CoreFoundation",
        "CoreGraphics",
        "CoreText",
        "CoreVideo",
        "Foundation",
        "IOSurface",
        "Metal",
        "QuartzCore",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}

fn compile_shim(manifest: &Path, source: &Path, out_dir: &Path, tools: &NativeTools) {
    let object = out_dir.join("ghostty_surface.o");
    let library = out_dir.join("libgpui_ghostty_surface.a");
    let status = tools
        .command(tools.xcode_tool("clang"))
        .args([
            "-c",
            "-fblocks",
            "-fno-objc-arc",
            "-isysroot",
            &tools.sdk_root,
            "-I",
            source
                .join("include")
                .to_str()
                .expect("UTF-8 Ghostty include path"),
            manifest
                .join("shim/ghostty_surface.m")
                .to_str()
                .expect("UTF-8 shim source path"),
            "-o",
            object.to_str().expect("UTF-8 shim object path"),
        ])
        .status()
        .expect("compile libghostty Objective-C shim");
    assert!(status.success(), "libghostty Objective-C shim failed");
    let status = tools
        .command(tools.xcode_tool("ar"))
        .args(["rcs", library.to_str().expect("UTF-8 shim library path")])
        .arg(&object)
        .status()
        .expect("archive libghostty Objective-C shim");
    assert!(status.success(), "libghostty Objective-C archive failed");
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=gpui_ghostty_surface");
}

struct NativeTools {
    developer_dir: String,
    sdk_root: String,
    zig: std::ffi::OsString,
}

impl NativeTools {
    fn detect() -> Self {
        Self {
            developer_dir: command_output("/usr/bin/xcode-select", &["-p"], &["DEVELOPER_DIR"]),
            sdk_root: command_output(
                "/usr/bin/xcrun",
                &["--sdk", "macosx", "--show-sdk-path"],
                &["DEVELOPER_DIR", "SDKROOT"],
            ),
            zig: env::var_os("ZIG").unwrap_or_else(|| "zig".into()),
        }
    }

    fn toolchain(&self) -> PathBuf {
        Path::new(&self.developer_dir).join("Toolchains/XcodeDefault.xctoolchain/usr/bin")
    }

    fn xcode_tool(&self, name: &str) -> PathBuf {
        self.toolchain().join(name)
    }

    fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        command
            .env_remove("NIX_CFLAGS_COMPILE")
            .env_remove("NIX_LDFLAGS")
            .env_remove("NIX_CC")
            .env_remove("NIX_BINTOOLS")
            .env("DEVELOPER_DIR", &self.developer_dir)
            .env("SDKROOT", &self.sdk_root);
        command
    }
}

fn build_ghostty(
    source: &Path,
    out_dir: &Path,
    prefix: &Path,
    fingerprint: &str,
    tools: &NativeTools,
) {
    let native_work = out_dir.join("native");
    let build_source = native_work.join(format!("ghostty-build-source-{fingerprint}"));
    let staging_prefix = native_work.join(format!("ghostty-prefix-{fingerprint}"));
    for stale in [&build_source, &staging_prefix] {
        if stale.exists() {
            std::fs::remove_dir_all(stale).expect("remove stale Ghostty build directory");
        }
    }
    copy_tree(source, &build_source);

    let toolchain = tools.toolchain();
    let path = format!(
        "{}:/usr/bin:/bin:{}",
        toolchain.display(),
        env::var("PATH").unwrap_or_default()
    );
    let package_cache = package_cache_dir(out_dir);
    std::fs::create_dir_all(&package_cache).expect("create shared Ghostty package cache");
    let source_package_cache = build_source.join("zig-pkg");
    let linked_package_cache = std::fs::symlink_metadata(&source_package_cache).is_err();
    if linked_package_cache {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&package_cache, &source_package_cache)
            .expect("link Ghostty package cache into the shared target directory");
    }

    let mut command = tools.command(&tools.zig);
    command
        .current_dir(&build_source)
        .env("CC", toolchain.join("clang"))
        .env("AR", toolchain.join("ar"))
        .env("LD", toolchain.join("ld"))
        .env("PATH", path)
        .env(
            "ZIG_GLOBAL_CACHE_DIR",
            shared_target_root(out_dir).join("ghostty-zig-global-cache"),
        )
        .env("ZIG_LOCAL_CACHE_DIR", native_work.join("ghostty-zig-cache"))
        .args(["build", "--prefix"])
        .arg(&staging_prefix)
        .args(GHOSTTY_BUILD_OPTIONS);
    let status = command.status();

    if linked_package_cache {
        std::fs::remove_file(&source_package_cache)
            .expect("remove temporary Ghostty package cache link");
    }
    let status = status.unwrap_or_else(|error| {
        cleanup_build(&build_source, &staging_prefix);
        panic!(
            "run Zig to build libghostty with {:?}: {error}; install Zig 0.16 or set ZIG",
            tools.zig
        )
    });
    if !status.success() {
        cleanup_build(&build_source, &staging_prefix);
        panic!("libghostty Zig build failed");
    }
    if !staging_prefix.join("lib/libghostty-internal.a").is_file() {
        cleanup_build(&build_source, &staging_prefix);
        panic!("libghostty Zig build did not produce its static archive");
    }

    std::fs::rename(&staging_prefix, prefix).expect("publish Ghostty native build");
    std::fs::remove_dir_all(build_source).expect("remove writable Ghostty build source");
}

fn cleanup_build(source: &Path, prefix: &Path) {
    let _ = std::fs::remove_dir_all(source);
    let _ = std::fs::remove_dir_all(prefix);
}

fn native_fingerprint(source: &Path, tools: &NativeTools) -> String {
    let zig_version = tools
        .command(&tools.zig)
        .arg("version")
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "read Zig version from {:?}: {error}; install Zig 0.16 or set ZIG",
                tools.zig
            )
        });
    assert!(zig_version.status.success(), "read Zig version");

    let mut hash = Fnv128::new();
    hash.write_field(NATIVE_CACHE_VERSION.as_bytes());
    hash.write_field(env::var("TARGET").expect("Cargo target triple").as_bytes());
    hash.write_field(&zig_version.stdout);
    hash.write_field(tools.developer_dir.as_bytes());
    hash.write_field(tools.sdk_root.as_bytes());
    for option in GHOSTTY_BUILD_OPTIONS {
        hash.write_field(option.as_bytes());
    }
    hash_tree(source, source, &mut hash);
    format!("{:032x}", hash.finish())
}

fn hash_tree(root: &Path, directory: &Path, hash: &mut Fnv128) {
    let mut entries: Vec<_> = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        .map(|entry| entry.expect("read vendored Ghostty source entry"))
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).expect("Ghostty source path");
        let file_type = entry
            .file_type()
            .unwrap_or_else(|error| panic!("read {} type: {error}", path.display()));
        hash.write_field(relative.to_string_lossy().as_bytes());
        if file_type.is_dir() {
            hash.write_field(b"directory");
            hash_tree(root, &path, hash);
        } else if file_type.is_file() {
            hash.write_field(b"file");
            let contents = std::fs::read(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            hash.write_field(&contents);
        } else {
            panic!(
                "unsupported entry in vendored Ghostty source: {}",
                path.display()
            );
        }
    }
}

struct Fnv128(u128);

impl Fnv128 {
    const OFFSET_BASIS: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

    const fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u128::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn write_field(&mut self, bytes: &[u8]) {
        self.write(&(bytes.len() as u64).to_le_bytes());
        self.write(bytes);
    }

    const fn finish(&self) -> u128 {
        self.0
    }
}

fn native_cache_root(out_dir: &Path) -> PathBuf {
    if let Some(path) = env::var_os("GHOSTTY_NATIVE_CACHE_DIR").map(PathBuf::from) {
        assert!(path.is_absolute(), "Ghostty native cache must be absolute");
        return path;
    }
    shared_target_root(out_dir).join("gpui-ghostty-native")
}

fn package_cache_dir(out_dir: &Path) -> PathBuf {
    if let Some(path) = env::var_os("GHOSTTY_ZIG_PACKAGE_CACHE_DIR").map(PathBuf::from) {
        assert!(
            path.is_absolute(),
            "Ghostty Zig package cache must be absolute"
        );
        return path;
    }
    shared_target_root(out_dir).join("ghostty-zig-pkg")
}

fn shared_target_root(out_dir: &Path) -> PathBuf {
    let package_dir = out_dir.parent();
    let build_dir = package_dir.and_then(Path::parent);
    let profile_dir = build_dir.and_then(Path::parent);
    if out_dir.file_name() == Some(OsStr::new("out"))
        && build_dir.and_then(Path::file_name) == Some(OsStr::new("build"))
        && let Some(root) = profile_dir.and_then(Path::parent)
    {
        return root.to_path_buf();
    }
    out_dir.join("shared-native-cache")
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).expect("create writable Ghostty source directory");
    for entry in std::fs::read_dir(source).expect("read vendored Ghostty source") {
        let entry = entry.expect("read vendored Ghostty source entry");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().expect("read Ghostty source entry type");
        if file_type.is_dir() {
            copy_tree(&source_path, &destination_path);
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &destination_path).expect("copy Ghostty source file");
        } else {
            panic!(
                "unsupported entry in vendored Ghostty source: {}",
                source_path.display()
            );
        }
    }
}

fn command_output(program: &str, args: &[&str], removed: &[&str]) -> String {
    let mut command = Command::new(program);
    command.args(args);
    for name in removed {
        command.env_remove(name);
    }
    let output = command.output().expect("run Xcode tool lookup");
    assert!(output.status.success(), "{program} failed");
    String::from_utf8(output.stdout)
        .expect("Xcode path is UTF-8")
        .trim()
        .to_owned()
}
