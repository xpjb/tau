#!/usr/bin/env python3
"""Small stdlib-only helpers for release-beta.sh; no builds or model calls."""
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
import urllib.request
import xml.etree.ElementTree as ET
import zipfile

if not __debug__:
    raise RuntimeError("Release verification must not run with Python assertion checks disabled")

ROOT = Path(__file__).resolve().parent.parent
ANDROID = '{http://schemas.android.com/apk/res/android}'


def run(*args, **kwargs):
    return subprocess.check_output(args, **kwargs)


def digest(path):
    with Path(path).open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def atomic(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile('w', dir=path.parent, delete=False) as f:
        json.dump(value, f, indent=2)
        f.write('\n')
        temporary = f.name
    os.replace(temporary, path)


def metadata():
    front = tomllib.loads((ROOT / 'frontend/Cargo.toml').read_text())['package']['version']
    daemon = tomllib.loads((ROOT / 'daemon/Cargo.toml').read_text())['package']['version']
    assert re.fullmatch(r'\d+\.\d+\.\d+', front) and front == daemon, 'Package version mismatch'
    manifest = ET.parse(ROOT / 'frontend/android/AndroidManifest.xml').getroot()
    assert manifest.get('package') == 'app.tau.rust'
    assert manifest.get(ANDROID + 'versionName') == front + '-beta', 'Android version mismatch'
    protocol = int(re.search(r'PROTOCOL_VERSION\s*:\s*u32\s*=\s*(\d+)', (ROOT / 'protocol/src/lib.rs').read_text())[1])
    return front, protocol, int(manifest.get(ANDROID + 'versionCode'))


def bump(version):
    old, _, code = metadata()
    assert re.fullmatch(r'\d+\.\d+\.\d+', version), 'Invalid version'
    if version == old:
        return
    assert tuple(map(int, version.split('.'))) > tuple(map(int, old.split('.'))), 'Version must increase'
    for name in ['daemon', 'frontend']:
        p = ROOT / name / 'Cargo.toml'
        p.write_text(p.read_text().replace(f'version = "{old}"', f'version = "{version}"', 1))
    p = ROOT / 'Cargo.lock'
    text = p.read_text()
    for name in ['taud', 'tau-frontend']:
        needle = f'name = "{name}"\nversion = "{old}"'
        assert text.count(needle) == 1
        text = text.replace(needle, f'name = "{name}"\nversion = "{version}"')
    p.write_text(text)
    p = ROOT / 'frontend/android/AndroidManifest.xml'
    p.write_text(p.read_text().replace(f'android:versionCode="{code}"', f'android:versionCode="{code + 1}"').replace(f'android:versionName="{old}-beta"', f'android:versionName="{version}-beta"'))


def fingerprint(stage):
    common = ['Cargo.toml', 'Cargo.lock', '.cargo/', 'rust-toolchain', 'blocks/', 'protocol/', 'transfer/']
    paths = {
        'daemon': common + ['daemon/', 'scripts/title_prompt.txt'],
        'windows': common + ['frontend/', 'markdown/', 'windows/', 'scripts/build-windows-sfx.sh'],
        'android': common + ['frontend/', 'markdown/', 'deploy/android-signing.sha256'],
        'check': common + ['daemon/', 'frontend/', 'markdown/', 'windows/', 'scripts/title_prompt.txt'],
        'test': common + ['daemon/', 'frontend/', 'markdown/', 'windows/', 'scripts/title_prompt.txt'],
    }[stage]
    h = hashlib.sha256()
    names = run('git', '-C', str(ROOT), 'ls-files', '-z').decode().split('\0')
    for name in sorted(filter(None, names)):
        p = ROOT / name
        if not any(name == prefix or name.startswith(prefix) for prefix in paths):
            continue
        if stage == 'windows' and name.startswith('frontend/android/'):
            continue
        if p.name in {'README.md', 'PACKAGING.md', 'QA.md', 'HANDOFF.md', 'ARCHITECTURE.md'} or name.startswith('frontend/docs/'):
            continue
        h.update(name.encode() + b'\0' + bytes.fromhex(digest(p)))
    # Build-affecting environment and the installed toolchain/config, not docs-only commits.
    for key, value in sorted(os.environ.items()):
        if key.startswith(('RUSTFLAGS', 'RUSTC', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_RUSTFLAGS', 'CARGO_PROFILE_', 'CARGO_TARGET_', 'ANDROID_', 'TAU_LZMA_', 'XWIN_', 'CC', 'AR', 'CFLAGS', 'CXXFLAGS', 'LDFLAGS')):
            h.update((key + '=' + value).encode())
    cargo_home = Path(os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')))
    for p in [cargo_home / 'config.toml', cargo_home / 'config', Path.home() / '.rustup/settings.toml']:
        if p.is_file():
            h.update(bytes.fromhex(digest(p)))
    # rustup's path lookup is read-only; never invokes the compiler outside the managed wrapper.
    tool = Path(run('rustup', 'which', 'rustc', text=True).strip())
    h.update(f'{tool}:{tool.stat().st_size}:{tool.stat().st_mtime_ns}'.encode())
    return h.hexdigest()


def receipt(path, key, files):
    atomic(path, {'key': key, 'files': {str(Path(p).resolve()): digest(p) for p in files}})


def hit(path, key):
    try:
        value = json.loads(Path(path).read_text())
        return value['key'] == key and all(digest(p) == sha for p, sha in value['files'].items())
    except (OSError, ValueError, KeyError):
        return False


def certificate(apk):
    tools = Path(os.environ.get('ANDROID_SDK_ROOT', os.environ.get('ANDROID_HOME', str(Path.home() / 'android-sdk')))) / 'build-tools' / os.environ.get('ANDROID_BUILD_TOOLS_VERSION', '35.0.0')
    result = run(str(tools / 'apksigner'), 'verify', '--print-certs', str(apk), text=True)
    signers = re.findall(r'Signer #\d+ certificate SHA-256 digest: (\w+)', result)
    assert len(signers) == 1, 'Exactly one beta signing identity required'
    return signers[0]


def check_key():
    key = Path(os.environ.get('ANDROID_KEYSTORE', str(Path.home() / '.android/debug.keystore')))
    assert key.is_file(), 'Existing beta signing key required; refusing to create another identity'
    der = run('keytool', '-exportcert', '-keystore', str(key), '-storepass', 'android', '-alias', 'androiddebugkey', stderr=subprocess.DEVNULL)
    assert hashlib.sha256(der).hexdigest() == (ROOT / 'deploy/android-signing.sha256').read_text().strip(), 'Wrong beta signing identity'


def verify_windows(version):
    setup = ROOT / f'dist/Tau-Beta-{version}-windows-x64.exe'
    payload = ROOT / f'target/windows-sfx-Tau-Beta-{version}/tau-windows-payload.tar.lzma'
    native = ROOT / 'target/x86_64-pc-windows-msvc/release/tau.exe'
    with lzma.open(payload, 'rb') as f, tarfile.open(fileobj=f, mode='r|') as archive:
        names = []
        for member in archive:
            names.append(member.name)
            assert member.isfile()
            with archive.extractfile(member) as body:
                assert hashlib.file_digest(body, 'sha256').hexdigest() == digest(native), 'Wrong installer app payload'
        assert names == ['app/Tau Beta.exe']
    with setup.open('rb') as f, mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ) as image:
        assert image[:2] == b'MZ' and image.find(payload.read_bytes()) >= 0, 'Installer payload not embedded'
        launcher = ROOT / 'windows/target/x86_64-pc-windows-msvc/release/tau-launcher.exe'
        assert image.find(launcher.read_bytes()) >= 0, 'Installer launcher not embedded'
    return setup


def verify_android(version, code):
    tools = Path(os.environ.get('ANDROID_SDK_ROOT', os.environ.get('ANDROID_HOME', str(Path.home() / 'android-sdk')))) / 'build-tools' / os.environ.get('ANDROID_BUILD_TOOLS_VERSION', '35.0.0')
    apk = ROOT / 'target/android/arm64-v8a/tau-frontend-arm64-v8a.apk'
    lib = ROOT / 'target/android/arm64-v8a/libtau_frontend.so'
    assert certificate(apk) == (ROOT / 'deploy/android-signing.sha256').read_text().strip(), 'APK signing identity changed'
    badging = run(str(tools / 'aapt2'), 'dump', 'badging', str(apk), text=True)
    assert f"name='app.tau.rust'" in badging.splitlines()[0]
    assert f"versionName='{version}-beta'" in badging.splitlines()[0] and f"versionCode='{code}'" in badging.splitlines()[0]
    assert 'application-debuggable' not in badging
    with zipfile.ZipFile(apk) as archive:
        assert archive.testzip() is None
        with archive.open('lib/arm64-v8a/libtau_frontend.so') as body:
            assert hashlib.file_digest(body, 'sha256').hexdigest() == digest(lib)
    loads = [line for line in run('readelf', '-lW', str(lib), text=True).splitlines() if line.strip().startswith('LOAD')]
    assert loads and all(int(line.split()[-1], 16) >= 16384 for line in loads)
    subprocess.run([str(tools / 'zipalign'), '-c', '-P', '16', '4', str(apk)], check=True)
    return apk


def health(version, protocol):
    with urllib.request.urlopen('http://127.0.0.1:8791/v1/health', timeout=3) as response:
        result = json.load(response)
    assert result['version'] == version and result['protocolVersion'] == int(protocol), 'Wrong running beta version/protocol'


def stop_owned(marker):
    # Cancel only managed service envelopes tagged by this invocation. Never a
    # global pkill, another Cargo job, beta/stable, or a wrapper-policy bypass.
    assert marker
    units = run('systemctl', 'list-units', '--type=service', '--state=running', 'run-*', '--no-legend', '--plain', text=True)
    for line in units.splitlines():
        unit = line.split()[0]
        try:
            pid = run('systemctl', 'show', unit, '-p', 'MainPID', '--value', text=True).strip()
        except subprocess.CalledProcessError:
            continue
        try:
            env = Path(f'/proc/{pid}/environ').read_bytes().split(b'\0')
        except OSError:
            continue
        if f'TAU_BETA_RELEASE_RUN={marker}'.encode() in env:
            subprocess.run(['systemctl', 'stop', unit], check=True)


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
        print(verify_android(args[0], int(args[1])))
    elif command == 'health':
        health(*args)
    elif command == 'stop-owned':
        stop_owned(args[0])
    else:
        raise ValueError('Unknown helper command')


if __name__ == '__main__':
    main()
