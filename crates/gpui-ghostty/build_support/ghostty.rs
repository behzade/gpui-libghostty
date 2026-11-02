//! Shared by build.rs and the release preparation tool. Published crates carry
//! pristine source; Git/path builds can obtain the same checksum-pinned archive.
use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
type Inventory = BTreeMap<String, String>;

#[derive(Deserialize)]
struct Pin {
    version: String,
    commit: String,
    url: String,
    sha256: String,
    prefix: String,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn safe_path(value: &str) -> Result<&Path> {
    let path = Path::new(value);
    if value.is_empty()
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(format!("invalid Ghostty source path: {value}").into());
    }
    Ok(path)
}

fn inventory(path: &Path) -> Result<Inventory> {
    let mut result = BTreeMap::new();
    for line in fs::read_to_string(path)?.lines() {
        let (hash, name) = line
            .split_once("  ")
            .ok_or("invalid source inventory line")?;
        safe_path(name)?;
        if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("invalid SHA-256 for {name}").into());
        }
        if result.insert(name.to_owned(), hash.to_owned()).is_some() {
            return Err(format!("duplicate source inventory entry: {name}").into());
        }
    }
    if result.is_empty() {
        return Err("empty Ghostty source inventory".into());
    }
    Ok(result)
}

fn verify(root: &Path, expected: &Inventory) -> Result<()> {
    fn walk(root: &Path, dir: &Path, actual: &mut Inventory) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                walk(root, &path, actual)?;
            } else if kind.is_file() {
                let name = path
                    .strip_prefix(root)?
                    .to_str()
                    .ok_or("non-UTF-8 source path")?;
                actual.insert(name.to_owned(), digest(&fs::read(&path)?));
            } else {
                return Err(format!("unexpected source entry: {}", path.display()).into());
            }
        }
        Ok(())
    }
    let mut actual = BTreeMap::new();
    walk(root, root, &mut actual)?;
    for name in expected.keys().chain(actual.keys()) {
        if expected.get(name) != actual.get(name) {
            return Err(format!(
                "Ghostty source checksum mismatch: {}",
                root.join(name).display()
            )
            .into());
        }
    }
    Ok(())
}

fn lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.lock()?;
    Ok(file)
}

fn archive(pin: &Pin, cache: &Path) -> Result<PathBuf> {
    let supplied = std::env::var_os("GHOSTTY_SOURCE_ARCHIVE").map(PathBuf::from);
    let path = supplied
        .clone()
        .unwrap_or_else(|| cache.join(format!("{}.tar.gz", pin.sha256)));
    if supplied.is_some() && !path.is_absolute() {
        return Err("GHOSTTY_SOURCE_ARCHIVE must be an absolute path".into());
    }
    if !path.exists() && supplied.is_none() {
        if std::env::var("CARGO_NET_OFFLINE").is_ok_and(|value| value == "true") {
            return Err("Ghostty source is not cached; supply GHOSTTY_SOURCE_ARCHIVE or prepare upstream/ghostty before building offline".into());
        }
        let temporary = path.with_extension("download");
        let result = (|| -> Result<()> {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(180)))
                .build()
                .into();
            let mut response = agent.get(&pin.url).call()?;
            let mut file = File::create(&temporary)?;
            // A corrupt or unexpected response must not fill the disk.
            io::copy(
                &mut response.body_mut().as_reader().take(512 * 1024 * 1024),
                &mut file,
            )?;
            if digest(&fs::read(&temporary)?) != pin.sha256 {
                return Err("downloaded Ghostty archive checksum mismatch".into());
            }
            fs::rename(&temporary, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
    }
    if digest(&fs::read(&path)?) != pin.sha256 {
        return Err(format!("Ghostty archive checksum mismatch: {}", path.display()).into());
    }
    Ok(path)
}

/// Materialize only the recorded build closure, keeping every file byte-for-byte
/// identical to upstream. Do not mutate an existing tree: verify it instead.
pub fn materialize(manifest: &Path, destination: &Path, cache: &Path) -> Result<()> {
    fs::create_dir_all(cache)?;
    let _lock = lock(&cache.join("source.lock"))?;
    let expected = inventory(&manifest.join("upstream/files.sha256"))?;
    if destination.exists() {
        return verify(destination, &expected);
    }
    let pin: Pin = serde_json::from_slice(&fs::read(manifest.join("upstream/ghostty.json"))?)?;
    if pin.commit.len() != 40 || !pin.url.starts_with("https://") || pin.version.is_empty() {
        return Err("invalid Ghostty source pin".into());
    }
    let archive = archive(&pin, cache)?;
    let staging =
        destination.with_file_name(format!(".ghostty-source-{}.staging", std::process::id()));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    let result = (|| -> Result<()> {
        let mut archive = tar::Archive::new(GzDecoder::new(File::open(archive)?));
        let mut found = Inventory::new();
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            let Ok(relative) = path.strip_prefix(&pin.prefix) else {
                continue;
            };
            let name = relative.to_str().ok_or("non-UTF-8 archive path")?;
            let Some(expected_hash) = expected.get(name) else {
                continue;
            };
            safe_path(name)?;
            if !entry.header().entry_type().is_file() || found.contains_key(name) {
                return Err(format!("unexpected archive entry: {name}").into());
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            if digest(&bytes) != *expected_hash {
                return Err(format!("upstream file checksum mismatch: {name}").into());
            }
            let output = staging.join(relative);
            fs::create_dir_all(output.parent().ok_or("source parent")?)?;
            fs::write(&output, bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    &output,
                    fs::Permissions::from_mode(entry.header().mode()? & 0o777),
                )?;
            }
            found.insert(name.to_owned(), expected_hash.clone());
        }
        if found != expected {
            return Err("Ghostty archive is missing files from the source inventory".into());
        }
        fs::rename(&staging, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// Return a verified, patched build tree. Neither the bundled source nor the
/// checkout source is changed, and no external patch/Git/Python tool is needed.
pub fn prepare(manifest: &Path, work: &Path, cache: &Path) -> Result<PathBuf> {
    let expected = inventory(&manifest.join("upstream/files.sha256"))?;
    let upstream = manifest.join("upstream/ghostty");
    let upstream = if upstream.exists() {
        verify(&upstream, &expected)?;
        upstream
    } else {
        let id = digest(&fs::read(manifest.join("upstream/files.sha256"))?);
        let source = cache.join(format!("upstream-{id}"));
        materialize(manifest, &source, cache)?;
        source
    };
    let patches = manifest.join("patches/ghostty");
    let mut patched_inventory = expected.clone();
    for (name, hash) in inventory(&patches.join("files.sha256"))? {
        if !expected.contains_key(&name) {
            return Err(format!("patched checksum is outside the source inventory: {name}").into());
        }
        patched_inventory.insert(name, hash);
    }
    let id = digest(&serde_json::to_vec(&patched_inventory)?);
    fs::create_dir_all(work)?;
    let _lock = lock(&work.join("patch.lock"))?;
    let destination = work.join(format!("ghostty-patched-{id}"));
    // Always apply the patches and check their output, even if native artifacts
    // are cached. A changed patch must never be silently ignored.
    let staging = work.join(format!(".ghostty-patched-{}.staging", std::process::id()));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    let result = (|| -> Result<()> {
        for name in expected.keys() {
            let output = staging.join(name);
            fs::create_dir_all(output.parent().ok_or("source parent")?)?;
            fs::copy(upstream.join(name), output)?;
        }
        for name in fs::read_to_string(patches.join("series"))?
            .lines()
            .filter(|line| !line.is_empty())
        {
            safe_path(name)?;
            let target = name
                .strip_suffix(".patch")
                .ok_or("patch must have .patch suffix")?;
            if !expected.contains_key(target) {
                return Err(
                    format!("patch target is outside the source inventory: {target}").into(),
                );
            }
            let patch = fs::read_to_string(patches.join(name))?;
            let patch = diffy::Patch::from_str(&patch)?;
            let original = fs::read_to_string(staging.join(target))?;
            let modified = diffy::apply(&original, &patch)?;
            fs::write(staging.join(target), modified)?;
        }
        verify(&staging, &patched_inventory)?;
        if destination.exists() {
            // Do not replace a tree another native build might be reading.
            verify(&destination, &patched_inventory)?;
            fs::remove_dir_all(&staging)?;
        } else {
            fs::rename(&staging, &destination)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("crate");
        let cache = temp.path().join("cache");
        fs::create_dir_all(manifest.join("upstream")).unwrap();
        fs::create_dir_all(manifest.join("patches/ghostty")).unwrap();
        fs::create_dir_all(&cache).unwrap();
        let mut tar = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        for (name, contents) in [
            ("ghostty-test/a.zig", "before\n"),
            ("ghostty-test/unneeded", "omit\n"),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, name, contents.as_bytes())
                .unwrap();
        }
        let bytes = tar.into_inner().unwrap().finish().unwrap();
        let hash = digest(&bytes);
        fs::write(cache.join(format!("{hash}.tar.gz")), bytes).unwrap();
        fs::write(
            manifest.join("upstream/ghostty.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": "test", "commit": "0".repeat(40), "url": "https://invalid.test/source",
                "sha256": hash, "prefix": "ghostty-test"
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            manifest.join("upstream/files.sha256"),
            format!("{}  a.zig\n", digest(b"before\n")),
        )
        .unwrap();
        fs::write(
            manifest.join("patches/ghostty/files.sha256"),
            format!("{}  a.zig\n", digest(b"after\n")),
        )
        .unwrap();
        fs::write(manifest.join("patches/ghostty/series"), "a.zig.patch\n").unwrap();
        fs::write(
            manifest.join("patches/ghostty/a.zig.patch"),
            diffy::create_patch("before\n", "after\n").to_string(),
        )
        .unwrap();
        (temp, manifest, cache)
    }

    #[test]
    fn archive_selection_and_patching_preserve_pristine_source() {
        let (temp, manifest, cache) = fixture();
        let source = manifest.join("upstream/ghostty");
        materialize(&manifest, &source, &cache).unwrap();
        assert!(!source.join("unneeded").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(source.join("a.zig"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
        }
        // Bundled source works with no archive/cache available, as in a crate.
        fs::remove_dir_all(&cache).unwrap();
        let patched = prepare(&manifest, &temp.path().join("work"), &cache).unwrap();
        assert_eq!(
            fs::read_to_string(source.join("a.zig")).unwrap(),
            "before\n"
        );
        assert_eq!(
            fs::read_to_string(patched.join("a.zig")).unwrap(),
            "after\n"
        );
        assert!(!cache.exists());
    }

    #[test]
    fn cached_output_does_not_hide_changed_or_invalid_patches() {
        let (temp, manifest, cache) = fixture();
        let work = temp.path().join("work");
        prepare(&manifest, &work, &cache).unwrap();
        fs::write(
            manifest.join("patches/ghostty/a.zig.patch"),
            diffy::create_patch("before\n", "wrong\n").to_string(),
        )
        .unwrap();
        assert!(
            prepare(&manifest, &work, &cache)
                .unwrap_err()
                .to_string()
                .contains("checksum mismatch")
        );
        fs::write(
            manifest.join("patches/ghostty/a.zig.patch"),
            diffy::create_patch("unmatched\n", "after\n").to_string(),
        )
        .unwrap();
        assert!(prepare(&manifest, &work, &cache).is_err());
    }

    #[test]
    fn changed_or_extra_upstream_files_are_rejected() {
        let (_temp, manifest, cache) = fixture();
        let source = manifest.join("upstream/ghostty");
        materialize(&manifest, &source, &cache).unwrap();
        fs::write(source.join("extra"), "unexpected").unwrap();
        assert!(materialize(&manifest, &source, &cache).is_err());
        fs::remove_file(source.join("extra")).unwrap();
        fs::write(source.join("a.zig"), "modified").unwrap();
        assert!(materialize(&manifest, &source, &cache).is_err());
    }

    #[test]
    fn corrupt_archive_is_rejected_before_source_is_written() {
        let (_temp, manifest, cache) = fixture();
        let pin: Pin =
            serde_json::from_slice(&fs::read(manifest.join("upstream/ghostty.json")).unwrap())
                .unwrap();
        fs::write(cache.join(format!("{}.tar.gz", pin.sha256)), "corrupt").unwrap();
        let source = manifest.join("upstream/ghostty");
        assert!(
            materialize(&manifest, &source, &cache)
                .unwrap_err()
                .to_string()
                .contains("archive checksum mismatch")
        );
        assert!(!source.exists());
    }
}
