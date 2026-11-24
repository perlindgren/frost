# Nice binary downloads

Each platform archive contains nine CLAP plugins, nine VST3 plugins, ten standalone
executables (including Nice Rack), and the accompanying licence and attribution
files. NAM models and cabinet impulse responses are not included. The games are
not part of these downloads. No Rust installation is needed to use the binaries.

## Choose a download

- `frost-nice-aarch64-apple-darwin.tar.gz`: Apple Silicon Macs (M1 or later).
  Requires macOS 14 or newer; built for the M1 CPU and a macOS 14 deployment target. The deployment target
  is not a substitute for testing on older macOS versions. Intel Macs are not
  supported by this download.
- `frost-nice-x86_64-pc-windows-msvc.tar.gz`: 64-bit Windows, x86-64-v3 CPU
  (including AVX2/FMA). The MSVC C runtime is statically linked; Windows system
  DLLs and installed audio drivers are still required. ASIO drivers are installed
  per system by the user; they are not bundled. No separate Visual C++
  Redistributable or Rust runtime installation should be needed.
- `frost-nice-x86_64-unknown-linux-gnu.tar.gz`: x86-64-v3 Linux, built on
  Ubuntu 22.04 (glibc 2.35 baseline). System audio, graphics and desktop libraries
  are dynamically linked. Consult the workflow's dependency inspection output
  for the exact libraries; this is not a fully static Linux distribution.

All application/plugin binaries use Cargo's release profile with optimization
level 3 and debug assertions disabled. The workspace's `log` dependency enables
`release_max_level_off`, removing calls made through its logging macros from
these optimized builds. This does not suppress direct stderr output, panics or
messages produced by native libraries. CPU flags are limited to this release
workflow; they do not change the engine or games' ordinary build defaults.

## Install and run

Extract the archive. On Windows, recent Windows versions provide `tar`; an
archive utility that supports `.tar.gz` also works. Keep the supplied notices.

Copy bundles from `plugins/` into your host's CLAP/VST3 search locations and ask
the host to rescan. Run the desired executable in `standalone/` for standalone
use. Nice Rack hosts CLAP plugins; select the extracted CLAP bundles in the rack.
Configure your input/output devices in each standalone application's settings.

On macOS these are executable files, not Finder `.app` bundles. They can be
launched from Terminal. Builds are ad-hoc signed, not Developer ID signed or
notarized; macOS security prompts may apply. Windows builds are currently
unsigned too. Signing/installers are separate future work.

Check an archive against `SHA256SUMS.txt` if desired. GitHub's separate
"Source code" downloads are repository snapshots, not the compiled downloads.

## Maintainer: testing and publishing

After merging the workflow into the default branch, open **Actions → Binary
release → Run workflow**, and select the branch to test. Successful jobs provide
three downloadable workflow artifacts, each containing the platform archive.
A manual run never creates or publishes a GitHub Release. GitHub initially needs
the workflow on the default branch for its manual-run button to be available.

For a release, tag the intended tested commit and push the tag, for example:

```sh
git tag -a v0.1.0 -m "Nice plugins v0.1.0"
git push upstream v0.1.0
```

Tag pushes matching `v[0-9]*` run all three builds. After all succeed, automation
creates a **draft** GitHub Release with the archives, checksums, this installation
information and generated change notes. Review and test the downloads, mark early
versions as prereleases if appropriate, then click **Publish release**. No binary
release is published automatically. A failed platform build prevents the draft
job from running. Retries can replace assets only while the release is a draft.
Use a new version tag to replace a published release.

The workflow pins Rust 1.100.0 and uses `--locked`. Hosted runner images and system
packages can still change; byte-for-byte reproducibility is not claimed. Release
builds validate legal-text provenance and archive contents, test the packaging
code, and inspect all 28 native binaries. Windows imports of a dynamic MSVC/UCRT
runtime or unreviewed DLL fail the build. macOS permits only system library load
commands and checks signatures. Linux fails on missing linked libraries and
prints its resolved dependencies. These checks do not replace audio/GUI testing
on actual machines or the normal pull-request test suite.
