use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

const GHOSTTY_COMMIT: &str = "9f0e1719";

fn main() {
    println!("cargo:rerun-if-changed=shim/ghostty_surface.m");
    println!("cargo:rerun-if-changed=../../vendor/ghostty/build.zig");
    println!("cargo:rerun-if-changed=../../vendor/ghostty/build.zig.zon");

    if env::var_os("CARGO_CFG_TARGET_OS").as_deref() != Some(std::ffi::OsStr::new("macos")) {
        return;
    }

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let source = manifest.join("../../vendor/ghostty");
    let target = cargo_target_dir(&manifest);
    let prefix = target.join(format!("libghostty-{GHOSTTY_COMMIT}"));
    let library = prefix.join("lib/libghostty-internal.a");

    if !library.exists() {
        build_ghostty(&source, &target, &prefix);
    }

    compile_shim(&manifest, &source);

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

fn compile_shim(manifest: &Path, source: &Path) {
    let developer_dir = command_output("/usr/bin/xcode-select", &["-p"], &["DEVELOPER_DIR"]);
    let sdk_root = command_output(
        "/usr/bin/xcrun",
        &["--sdk", "macosx", "--show-sdk-path"],
        &["DEVELOPER_DIR", "SDKROOT"],
    );
    let toolchain = Path::new(&developer_dir).join("Toolchains/XcodeDefault.xctoolchain/usr/bin");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    let object = out_dir.join("ghostty_surface.o");
    let library = out_dir.join("libgpui_ghostty_surface.a");
    let status = clean_xcode_command(&toolchain.join("clang"), &developer_dir, &sdk_root)
        .args([
            "-c",
            "-fblocks",
            "-fno-objc-arc",
            "-isysroot",
            &sdk_root,
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
    let status = clean_xcode_command(&toolchain.join("ar"), &developer_dir, &sdk_root)
        .args(["rcs", library.to_str().expect("UTF-8 shim library path")])
        .arg(&object)
        .status()
        .expect("archive libghostty Objective-C shim");
    assert!(status.success(), "libghostty Objective-C archive failed");
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=gpui_ghostty_surface");
}

fn clean_xcode_command(program: &Path, developer_dir: &str, sdk_root: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env_remove("NIX_CFLAGS_COMPILE")
        .env_remove("NIX_LDFLAGS")
        .env_remove("NIX_CC")
        .env_remove("NIX_BINTOOLS")
        .env("DEVELOPER_DIR", developer_dir)
        .env("SDKROOT", sdk_root);
    command
}

fn cargo_target_dir(manifest: &Path) -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| manifest.join("../../target"))
}

fn build_ghostty(source: &Path, target: &Path, prefix: &Path) {
    let developer_dir = command_output("/usr/bin/xcode-select", &["-p"], &["DEVELOPER_DIR"]);
    let sdk_root = command_output(
        "/usr/bin/xcrun",
        &["--sdk", "macosx", "--show-sdk-path"],
        &["DEVELOPER_DIR", "SDKROOT"],
    );
    let toolchain = Path::new(&developer_dir).join("Toolchains/XcodeDefault.xctoolchain/usr/bin");
    let zig = env::var_os("ZIG").unwrap_or_else(|| "zig".into());
    let path = format!(
        "{}:/usr/bin:/bin:{}",
        toolchain.display(),
        env::var("PATH").unwrap_or_default()
    );
    let package_cache = target.join("ghostty-zig-pkg");
    std::fs::create_dir_all(&package_cache).expect("create shared Ghostty package cache");
    let source_package_cache = source.join("zig-pkg");
    let linked_package_cache = std::fs::symlink_metadata(&source_package_cache).is_err();
    if linked_package_cache {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&package_cache, &source_package_cache)
            .expect("link Ghostty package cache into the shared target directory");
    }

    let status = Command::new(zig)
        .current_dir(source)
        .env_remove("NIX_CFLAGS_COMPILE")
        .env_remove("NIX_LDFLAGS")
        .env_remove("NIX_CC")
        .env_remove("NIX_BINTOOLS")
        .env("DEVELOPER_DIR", &developer_dir)
        .env("SDKROOT", &sdk_root)
        .env("CC", toolchain.join("clang"))
        .env("AR", toolchain.join("ar"))
        .env("LD", toolchain.join("ld"))
        .env("PATH", path)
        .env("ZIG_GLOBAL_CACHE_DIR", target.join("zig-global-cache"))
        .env("ZIG_LOCAL_CACHE_DIR", target.join("ghostty-zig-cache"))
        .args([
            "build",
            "--prefix",
            prefix.to_str().expect("UTF-8 Ghostty install prefix"),
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
        ])
        .status()
        .expect("run Zig to build libghostty");
    if linked_package_cache {
        std::fs::remove_file(&source_package_cache)
            .expect("remove temporary Ghostty package cache link");
    }
    assert!(status.success(), "libghostty Zig build failed");
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
