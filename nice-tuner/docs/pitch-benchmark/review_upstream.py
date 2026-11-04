#!/usr/bin/env python3
"""Run published source tests without copying or editing dependency source.

The small review manifests below point directly at the downloaded source. This
keeps generated lockfiles and build products out of dependency directories that
lack a packaged lockfile. Benchmark-only upstream dev dependencies are omitted.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib

HERE = Path(__file__).resolve().parent
STUDY = Path(tempfile.mkdtemp(prefix="frost-pitch-review-"))
ENV = os.environ.copy()
ENV["CARGO_HOME"] = str(STUDY / "cargo-home")
ENV["CARGO_TARGET_DIR"] = str(STUDY / "target")
PACKAGES = {"pitch-estimate": "0.1.0", "pitch-detection": "0.3.0", "pitch-detector": "0.3.1", "pitch": "0.2.0"}


def run(args, log):
    with (STUDY / log).open("w") as output:
        result = subprocess.run(args, env=ENV, stdout=output, stderr=subprocess.STDOUT, check=False)
    return {"exit_code": result.returncode, "log": log}


subprocess.run(["cargo", "fetch", "--locked", "--manifest-path", str(HERE / "Cargo.toml")], env=ENV, check=True)
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1", "--manifest-path", str(HERE / "Cargo.toml")], env=ENV))
results = {}
for name, version in PACKAGES.items():
    package = next(p for p in metadata["packages"] if p["name"] == name and p["version"] == version)
    source = Path(package["manifest_path"]).parent
    original = (source / "Cargo.toml").read_text()
    config = tomllib.loads(original)
    if name == "pitch-estimate":
        manifest = source / "Cargo.toml"  # Includes its published Cargo.lock.
    else:
        root = STUDY / (name + "-tests")
        root.mkdir()
        begin = original.find("[dependencies.")
        end = original.find("[dev-dependencies.", begin)
        dependencies = original[begin:end] if begin >= 0 else ""
        features = original[original.index("[features]"):] if "[features]" in original else ""
        crate_name = name.replace("-", "_")
        text = f'[package]\nname="review-{name}"\nversion="0.0.0"\nedition="{config["package"]["edition"]}"\n[workspace]\n[lib]\nname="{crate_name}"\npath="{source / "src/lib.rs"}"\n'
        if name == "pitch-detection":
            (root / "tests").mkdir()
            (root / "tests/samples").symlink_to(source / "tests/samples", target_is_directory=True)
            text += f'[[test]]\nname="published_tests"\npath="{source / "tests/main.rs"}"\n'
            dev = '[dev-dependencies]\nhound="3.4.0"\n'
        elif name == "pitch-detector":
            dev = '[dev-dependencies]\nfloat-cmp="0.9.0"\nserde={version="1.0.133",features=["derive"]}\nserde_json="1.0.74"\n'
        else:
            dev = ""
        manifest = root / "Cargo.toml"
        manifest.write_text(text + dependencies + dev + features)
    base = ["--manifest-path", str(manifest)]
    if name == "pitch-estimate":
        base += ["--locked"]
    results[name] = {
        "release_tests": run(["cargo", "test", *base, "--release"], name + "-release.log"),
        "debug_tests": run(["cargo", "test", *base], name + "-debug.log"),
        "clippy": run(["cargo", "clippy", *base, "--all-targets", "--", "-D", "warnings"], name + "-clippy.log"),
    }
    archive = Path(ENV["CARGO_HOME"]) / "registry/cache" / source.parent.name / f"{name}-{version}.crate"
    modified = []
    with tarfile.open(archive) as tar:
        for entry in tar.getmembers():
            if entry.isfile():
                relative = Path(entry.name).relative_to(f"{name}-{version}")
                if (source / relative).read_bytes() != tar.extractfile(entry).read():
                    modified.append(str(relative))
    results[name]["modified_source_files"] = modified
    results[name]["archive_sha256"] = hashlib.sha256(archive.read_bytes()).hexdigest()
(STUDY / "review-results.json").write_text(json.dumps(results, indent=2) + "\n")
print(STUDY)
print(json.dumps(results, indent=2))
