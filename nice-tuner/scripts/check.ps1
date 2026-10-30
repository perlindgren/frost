# Windows transcription of check.sh — run from the workspace root with:
#   [ScriptBlock]::Create((Get-Content nice-tuner\scripts\check.ps1 -Raw)).Invoke()
# (the ScriptBlock dance bypasses machine AllSigned policy).
#
# CPAL_ASIO_DIR no longer needs to be set by hand: `.cargo/config.toml` at the
# workspace root provides it. Point it at an ASIO SDK kept outside the tree —
# the SDK is proprietary and this repository is MIT/Apache, and the pre-commit
# hook refuses an `asiosdk` that grows into the tree anyway.
#
# Why this exists next to check.sh:
# - msys bash cannot initialize inside confined shells (0xC0000142), and
#   plain `bash` on PATH is WSL's, which is E_ACCESSDENIED there;
# - asio-sys's build script wants the unpacked ASIO SDK and otherwise dials
#   steinberg.net.
# Bundling is no longer skipped, as it was when this file was written: xtask's
# dependency graph (goblin via nice-plug-xtask) is in the workspace lock now,
# so the step runs here like anywhere else. What it produces has still never
# been loaded into a real host — see the crt-static note in
# `.cargo/config.toml` — which is why the step builds with RUSTFLAGS cleared
# and why a DAW still has to be the judge.
$ErrorActionPreference = 'Continue'
$env:RUSTFLAGS = ""
$log = Join-Path $env:TEMP "nice-tuner-gate.log"
Set-Content -Path $log -Value "gate transcript $(Get-Date -Format o)"

function Step {
    param([string]$name, [string]$cmdline)
    Add-Content $log "`n`n== $name =="
    cmd /c "($cmdline) >> `"$log`" 2>&1"
    $code = $LASTEXITCODE
    Write-Output ("step [{0}] -> exit {1}" -f $name, $code)
    if ($code -ne 0) { exit $code }
}

Step "cargo fmt" "cargo fmt --all -- --check"
Step "clippy plugin" "cargo clippy -p nice-tuner -p pitch -p xtask --all-targets -- -D warnings"
Step "clippy standalone" "cargo clippy -p nice-tuner --features standalone --all-targets -- -D warnings"
Step "clippy standalone-asio" "cargo clippy -p nice-tuner --features standalone-asio --all-targets -- -D warnings"
Step "plugin lib check" "cargo check -p nice-tuner"
Step "standalone check" "cargo check -p nice-tuner --features standalone"
if (rustup target list --installed 2>$null | Select-String -Quiet 'x86_64-pc-windows-msvc') {
    Step "clippy windows target" "cargo clippy -p nice-tuner --features standalone --target x86_64-pc-windows-msvc -- -D warnings"
} else {
    Write-Output "skipped: rustup target x86_64-pc-windows-msvc not installed"
}
Step "tests" "cargo test -p nice-tuner -p pitch"
Add-Content $log "`n`n== no event loop of ours =="
# The engine keeps winit (`run()` is its front door); what must stay true is
# that the guest never drives one. So the grep reads our own sources.
$hits = Get-ChildItem nice-tuner\src -Recurse -Filter *.rs |
    Select-String -Pattern 'EventLoop|ActiveEventLoop|ApplicationHandler|WindowAttributes' |
    Where-Object { $_.Line -notmatch '//' }
if ($hits) {
    $hits | ForEach-Object { Add-Content $log "$($_.Path):$($_.LineNumber): $($_.Line)" }
    Write-Output "step [winit grep] -> FAIL"
    exit 1
}
Write-Output "step [winit grep] -> exit 0"
Step "standalone bin build (linker gate)" "cargo build -p nice-tuner --features standalone --bin nice-tuner-standalone"
Step "plugin bundling" "cargo xtask bundle nice-tuner --release"
Write-Output "ALL GATES PASSED (full log: $log)"
