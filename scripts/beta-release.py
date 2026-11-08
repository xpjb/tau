#!/usr/bin/env python3
"""Version, fingerprint and package verification helpers for Tau client releases."""
import hashlib
import json
import lzma
import mmap
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import xml.etree.ElementTree as ET
import zipfile

if not __debug__:
    raise RuntimeError("Release verification must not run with Python assertion checks disabled")

ROOT = Path(__file__).resolve().parent.parent
ANDROID = '{http://schemas.android.com/apk/res/android}'


def run(*args, **kwargs):
    return subprocess.check_output(args, **kwargs)


def digest(path):
    with Path(path).open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def atomic(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile('w', dir=path.parent, delete=False) as output:
        json.dump(value, output, indent=2)
        output.write('\n')
        temporary = output.name
    os.replace(temporary, path)


def metadata():
    frontend = tomllib.loads((ROOT / 'crates/frontend/Cargo.toml').read_text())['package']['version']
    daemon = tomllib.loads((ROOT / 'crates/daemon/Cargo.toml').read_text())['package']['version']
    assert re.fullmatch(r'\d+\.\d+\.\d+', frontend) and frontend == daemon, 'Package version mismatch'
    manifest = ET.parse(ROOT / 'crates/frontend/android/AndroidManifest.xml').getroot()
    assert manifest.get('package') == 'app.tau.rust'
    assert manifest.get(ANDROID + 'versionName') == frontend + '-beta', 'Android version mismatch'
    protocol = int(re.search(r'PROTOCOL_VERSION\s*:\s*u32\s*=\s*(\d+)', (ROOT / 'crates/net/src/lib.rs').read_text())[1])
    return frontend, protocol, int(manifest.get(ANDROID + 'versionCode'))


def bump(version):
    old, _, code = metadata()
    assert re.fullmatch(r'\d+\.\d+\.\d+', version), 'Invalid version'
    if version == old:
        return
    assert tuple(map(int, version.split('.'))) > tuple(map(int, old.split('.'))), 'Version must increase'
    for name in ['daemon', 'frontend']:
        path = ROOT / 'crates' / name / 'Cargo.toml'
        path.write_text(path.read_text().replace(f'version = "{old}"', f'version = "{version}"', 1))
    path = ROOT / 'Cargo.lock'
    text = path.read_text()
    for name in ['taud', 'tau-frontend']:
        needle = f'name = "{name}"\nversion = "{old}"'
        assert text.count(needle) == 1
        text = text.replace(needle, f'name = "{name}"\nversion = "{version}"')
    path.write_text(text)
    path = ROOT / 'crates/frontend/android/AndroidManifest.xml'
    path.write_text(path.read_text().replace(f'android:versionCode="{code}"', f'android:versionCode="{code + 1}"').replace(f'android:versionName="{old}-beta"', f'android:versionName="{version}-beta"'))


def fingerprint(stage):
    common = ['Cargo.toml', 'Cargo.lock', '.cargo/', 'rust-toolchain', 'crates/block-store/', 'crates/net/', 'crates/code-viewer/', 'scripts/beta-release.py']
    paths = {
        'windows': common + ['crates/frontend/', 'crates/windows/', 'scripts/build-windows.sh'],
        'android': common + ['crates/frontend/', 'assets/android-signing.sha256', 'scripts/build-android.sh'],
    }[stage]
    result = hashlib.sha256()
    names = run('git', '-C', str(ROOT), 'ls-files', '-z').decode().split('\0')
    for name in sorted(filter(None, names)):
        path = ROOT / name
        if not any(name == prefix or name.startswith(prefix) for prefix in paths):
            continue
        if stage == 'windows' and name.startswith('crates/frontend/android/'):
            continue
        if path.suffix == '.md':
            continue
        result.update(name.encode() + b'\0' + bytes.fromhex(digest(path)))
    for key, value in sorted(os.environ.items()):
        if key.startswith(('RUSTFLAGS', 'RUSTC', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_RUSTFLAGS', 'CARGO_PROFILE_', 'CARGO_TARGET_', 'ANDROID_', 'TAU_LZMA_', 'XWIN_', 'CC', 'AR', 'CFLAGS', 'CXXFLAGS', 'LDFLAGS')):
            result.update((key + '=' + value).encode())
    cargo_home = Path(os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')))
    for path in [cargo_home / 'config.toml', cargo_home / 'config', Path.home() / '.rustup/settings.toml']:
        if path.is_file():
            result.update(bytes.fromhex(digest(path)))
    tool = Path(run('rustup', 'which', 'rustc', text=True).strip())
    result.update(f'{tool}:{tool.stat().st_size}:{tool.stat().st_mtime_ns}'.encode())
    return result.hexdigest()


def receipt(path, key, files):
    atomic(path, {'key': key, 'files': {str(Path(file).resolve()): digest(file) for file in files}})


def hit(path, key):
    try:
        value = json.loads(Path(path).read_text())
        return value['key'] == key and all(digest(file) == sha for file, sha in value['files'].items())
    except (OSError, ValueError, KeyError):
        return False


def certificate(apk):
    sdk = Path(os.environ.get('ANDROID_SDK_ROOT', os.environ.get('ANDROID_HOME', str(Path.home() / 'android-sdk'))))
    tools = sdk / 'build-tools' / os.environ.get('ANDROID_BUILD_TOOLS_VERSION', '35.0.0')
    result = run(str(tools / 'apksigner'), 'verify', '--print-certs', str(apk), text=True)
    signers = re.findall(r'Signer #\d+ certificate SHA-256 digest: (\w+)', result)
    assert len(signers) == 1, 'Exactly one beta signing identity required'
    return signers[0]


def check_key():
    key = Path(os.environ.get('ANDROID_KEYSTORE', str(Path.home() / '.android/debug.keystore')))
    assert key.is_file(), 'Existing beta signing key required'
    der = run('keytool', '-exportcert', '-keystore', str(key), '-storepass', 'android', '-alias', 'androiddebugkey', stderr=subprocess.DEVNULL)
    assert hashlib.sha256(der).hexdigest() == (ROOT / 'assets/android-signing.sha256').read_text().strip(), 'Wrong beta signing identity'


def verify_windows(version):
    setup = ROOT / f'dist/Tau-Beta-{version}-windows-x64.exe'
    payload = ROOT / f'target/windows-sfx-Tau-Beta-{version}/tau-windows-payload.tar.lzma'
    native = ROOT / 'target/x86_64-pc-windows-msvc/release/tau.exe'
    with lzma.open(payload, 'rb') as source, tarfile.open(fileobj=source, mode='r|') as archive:
        names = []
        for member in archive:
            names.append(member.name)
            assert member.isfile()
            with archive.extractfile(member) as body:
                assert hashlib.file_digest(body, 'sha256').hexdigest() == digest(native), 'Wrong installer app payload'
        assert names == ['app/Tau Beta.exe']
    with setup.open('rb') as source, mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ) as image:
        assert image[:2] == b'MZ' and image.find(payload.read_bytes()) >= 0, 'Installer payload not embedded'
        launcher = ROOT / 'crates/windows/target/x86_64-pc-windows-msvc/release/tau-launcher.exe'
        assert image.find(launcher.read_bytes()) >= 0, 'Installer launcher not embedded'
    return setup


def verify_android(version, code, abi):
    sdk = Path(os.environ.get('ANDROID_SDK_ROOT', os.environ.get('ANDROID_HOME', str(Path.home() / 'android-sdk'))))
    tools = sdk / 'build-tools' / os.environ.get('ANDROID_BUILD_TOOLS_VERSION', '35.0.0')
    apk = ROOT / f'target/android/{abi}/tau-frontend-{abi}.apk'
    library = ROOT / f'target/android/{abi}/libtau_frontend.so'
    assert certificate(apk) == (ROOT / 'assets/android-signing.sha256').read_text().strip(), 'APK signing identity changed'
    badging = run(str(tools / 'aapt2'), 'dump', 'badging', str(apk), text=True)
    assert "name='app.tau.rust'" in badging.splitlines()[0]
    assert f"versionName='{version}-beta'" in badging.splitlines()[0] and f"versionCode='{code}'" in badging.splitlines()[0]
    assert 'application-debuggable' not in badging
    with zipfile.ZipFile(apk) as archive:
        assert archive.testzip() is None
        with archive.open(f'lib/{abi}/libtau_frontend.so') as body:
            assert hashlib.file_digest(body, 'sha256').hexdigest() == digest(library)
    loads = [line for line in run('readelf', '-lW', str(library), text=True).splitlines() if line.strip().startswith('LOAD')]
    assert loads and all(int(line.split()[-1], 16) >= 16384 for line in loads)
    subprocess.run([str(tools / 'zipalign'), '-c', '-P', '16', '4', str(apk)], check=True)
    return apk


def main():
    command, *args = sys.argv[1:]
    if command == 'meta':
        print(*metadata())
    elif command == 'bump':
        bump(args[0])
    elif command == 'key':
        print(fingerprint(args[0]))
    elif command == 'stamp':
        receipt(args[0], args[1], args[2:])
    elif command == 'hit':
        sys.exit(0 if hit(*args) else 1)
    elif command == 'check-key':
        check_key()
    elif command == 'windows':
        print(verify_windows(args[0]))
    elif command == 'android':
        print(verify_android(args[0], int(args[1]), args[2] if len(args) > 2 else 'arm64-v8a'))
    else:
        raise ValueError('Unknown helper command')


if __name__ == '__main__':
    main()
