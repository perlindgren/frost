#!/usr/bin/env python3
"""Build release-only Nice downloads and inspect their native dependencies."""
import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
PACKAGES = ('nice-tuner', 'nice-nam', 'nice-dist', 'nice-phase', 'nice-clone',
            'nice-echo', 'nice-ecomplex', 'nice-wah', 'nice-pi-face', 'nice-rack')
FLAGS = {
    'aarch64-apple-darwin': '-Ctarget-cpu=apple-m1',
    'x86_64-pc-windows-msvc': '-Ctarget-cpu=x86-64-v3 -Ctarget-feature=+crt-static',
    'x86_64-unknown-linux-gnu': '-Ctarget-cpu=x86-64-v3',
}


def run(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True, stderr=subprocess.STDOUT)


def windows_imports(output):
    return set(re.findall(r'^\s+([\w.-]+\.dll)\s*$', output, re.I | re.M))


def check_windows_imports(imports):
    # OS components remain dynamic; third-party runtimes must not be required.
    # BCryptPrimitives is the built-in Windows ProcessPrng provider:
    # https://learn.microsoft.com/en-us/windows/win32/seccng/processprng
    # ComBase provides Windows COM/WinRT activation APIs:
    # https://learn.microsoft.com/en-us/windows/win32/api/roapi/nf-roapi-roregisteractivationfactories
    system = set('kernel32 user32 gdi32 gdi32full advapi32 ole32 oleaut32 shell32 '
                 'shlwapi uuid ws2_32 bcrypt bcryptprimitives ncrypt crypt32 secur32 winmm imm32 '
                 'combase comdlg32 comctl32 dwmapi dwrite d2d1 d3d11 d3d12 dxgi dxguid '
                 'opengl32 version setupapi cfgmgr32 avrt propsys winspool winhttp '
                 'wininet dbghelp psapi powrprof hid usp10 msimg32 uxtheme ntdll '
                 'rpcrt4 normaliz cabinet msvcrt mmdevapi windowscodecs runtimeobject '
                 'dcomp d3dcompiler_47 win32u msctf wintrust cryptbase msasn1 '
                 'dnsapi iphlpapi userenv netapi32 wtsapi32'.split())
    errors = []
    for name in sorted(imports):
        stem = name.lower().removesuffix('.dll')
        if stem.startswith(('vcruntime', 'msvcp', 'ucrtbase', 'api-ms-win-crt')):
            errors.append(f'Dynamic MSVC/UCRT runtime dependency: {name}')
            continue
        if stem not in system and not stem.startswith(('api-ms-win-', 'ext-ms-win-')):
            errors.append(f'Unreviewed external DLL dependency: {name}')
    if errors:
        raise RuntimeError('\n'.join(errors))


def check_windows_binaries(binaries):
    errors = []
    for name, imports in binaries:
        try:
            check_windows_imports(imports)
        except RuntimeError as error:
            errors.append(f'{name}:\n{error}')
    if errors:
        raise RuntimeError('Windows dependency verification failed:\n' + '\n'.join(errors))


def main():
    os.chdir(ROOT)
    manifest = tomllib.loads((ROOT / 'Cargo.toml').read_text())
    if 'release_max_level_off' not in manifest['dependencies']['log']['features']:
        raise RuntimeError('Release builds must compile out log macros')
    if os.environ.get('CARGO_ENCODED_RUSTFLAGS'):
        raise RuntimeError('CARGO_ENCODED_RUSTFLAGS would override release CPU/runtime flags')
    target = os.environ['RELEASE_TARGET']
    if os.environ.get('RUSTFLAGS') != FLAGS[target]:
        raise RuntimeError('Release RUSTFLAGS do not match the reviewed CPU/runtime policy')
    if os.environ.get('CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS', 'false') not in ('false', '0'):
        raise RuntimeError('Release debug assertions would re-enable logging')
    # Prevent Cargo config or environment from silently lowering release optimization.
    os.environ['CARGO_PROFILE_RELEASE_OPT_LEVEL'] = '3'
    os.environ['CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS'] = 'false'
    os.environ['CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS'] = 'false'
    print(run('rustc', '-vV'), flush=True)
    print(run('rustc', '--print', 'cfg', '--target', target, *FLAGS[target].split()), flush=True)
    args = ['cargo', 'run', '--locked', '-p', 'xtask', '--target', target, '--', 'package-release']
    for package in PACKAGES:
        args.extend(('-p', package))
    subprocess.run([*args, '--release', '--target', target, '--locked'], cwd=ROOT, check=True)
    output = Path(os.environ.get('CARGO_TARGET_DIR', 'target')) / 'downloads'
    directory = output / f'frost-nice-{target}'
    report = []
    windows_binaries = []
    count = 0
    for path in sorted(directory.rglob('*')):
        if not path.is_file():
            continue
        with path.open('rb') as binary:
            magic = binary.read(4)
        relative = str(path.relative_to(directory))
        if magic[:2] == b'MZ':
            imports = windows_imports(run('dumpbin', '/DEPENDENTS', str(path)))
            if not imports:
                raise RuntimeError(f'Could not read PE imports: {path}')
            windows_binaries.append((relative, imports))
            report.append(relative + ': ' + ', '.join(sorted(imports)))
        elif magic in (b'\xcf\xfa\xed\xfe', b'\xfe\xed\xfa\xcf'):
            # LC_ID_DYLIB is an identity, not an external dependency.
            commands = run('otool', '-l', str(path))
            for block in commands.split('Load command ')[1:]:
                if re.search(r'cmd LC_(?:LOAD|LOAD_WEAK|REEXPORT|LAZY_LOAD|LOAD_UPWARD)_DYLIB\b', block):
                    name = re.search(r'\bname (.*?) \(offset', block).group(1)
                    if not name.startswith(('/usr/lib/', '/System/Library/')):
                        raise RuntimeError(f'Non-system macOS dependency: {path}: {name}')
            subprocess.run(['codesign', '--verify', '--strict', str(path)], check=True)
            report.append(relative + ': system frameworks/libraries only')
        elif magic == b'\x7fELF':
            dependencies = run('ldd', str(path))
            if 'not found' in dependencies:
                raise RuntimeError(f'Missing Linux library: {path}\n{dependencies}')
            report.append(relative + ':\n' + dependencies)
        else:
            continue
        count += 1
    # Nine plugins in two formats, plus ten standalone executables.
    if count != 28:
        raise RuntimeError(f'Expected 28 native binaries, inspected {count}')
    print('\n'.join(report), flush=True)
    check_windows_binaries(windows_binaries)
    archive = output / f'frost-nice-{target}.tar.gz'
    subprocess.run([sys.executable, 'nice-tuner/xtask/scripts/check-release-licenses.py',
                    str(archive)], cwd=ROOT, check=True)
    print(f'SHA256 {hashlib.file_digest(archive.open("rb"), "sha256").hexdigest()} {archive.name}')


if __name__ == '__main__':
    main()
