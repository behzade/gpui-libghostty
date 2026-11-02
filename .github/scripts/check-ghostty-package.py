#!/usr/bin/env python3
"""Verify that a published crate contains its exact, pristine Ghostty source."""
import hashlib
from pathlib import Path
import sys
import tarfile

archive = Path(sys.argv[1])
crate = Path(__file__).resolve().parents[2] / "crates/gpui-ghostty"
expected = {
    name: digest
    for digest, name in (
        line.split("  ", 1)
        for line in (crate / "upstream/files.sha256").read_text().splitlines()
    )
}
with tarfile.open(archive, "r:gz") as package:
    actual = {}
    files = {}
    for entry in package:
        if not entry.isfile():
            continue
        relative = entry.name.split("/", 1)[1]
        data = package.extractfile(entry).read()
        files[relative] = hashlib.sha256(data).hexdigest()
        if relative.startswith("upstream/ghostty/"):
            actual[relative.removeprefix("upstream/ghostty/")] = files[relative]
        assert not relative.startswith("vendor/"), entry.name
    assert actual == expected, "crate is missing pristine Ghostty source or contains changed files"
    # Check the inputs which control source verification and patch application.
    for directory in ["patches", "build_support"]:
        for source in (crate / directory).rglob("*"):
            if source.is_file():
                name = source.relative_to(crate).as_posix()
                assert files.get(name) == hashlib.sha256(source.read_bytes()).hexdigest(), name
    for name in ["build.rs", "upstream/ghostty.json", "upstream/files.sha256", "upstream/LICENSE"]:
        assert files.get(name) == hashlib.sha256((crate / name).read_bytes()).hexdigest(), name
print(f"Verified {len(actual)} pristine Ghostty files and local patches in {archive}")
