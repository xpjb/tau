#!/usr/bin/env python3
"""Fast isolated tests for Tau release metadata and orchestration."""
import importlib.util
import io
import json
import lzma
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('release', Path(__file__).resolve().parents[1] / 'scripts/beta-release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.old = release.ROOT
        release.ROOT = self.root

    def tearDown(self):
        release.ROOT = self.old
        self.temp.cleanup()

    def write(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data.encode() if isinstance(data, str) else data)
        return path

    def test_receipts_require_exact_inputs_and_unchanged_artifacts(self):
        body = self.write('app.exe', b'MZoriginal')
        record = self.root / 'receipt.json'
        release.receipt(record, 'inputs', [body])
        self.assertTrue(release.hit(record, 'inputs'))
        self.assertFalse(release.hit(record, 'other-inputs'))
        body.write_bytes(b'MZmodified')
        self.assertFalse(release.hit(record, 'inputs'))
        body.unlink()
        self.assertFalse(release.hit(record, 'inputs'))

    def test_windows_fingerprint_ignores_docs_and_android_packaging_only(self):
        for name in ['Cargo.toml', 'Cargo.lock', 'scripts/beta-release.py', 'scripts/build-windows.sh', 'scripts/build-android.sh', 'crates/frontend/src/lib.rs', 'crates/frontend/android/AndroidManifest.xml', 'README.md']:
            self.write(name, 'initial')
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)
        subprocess.run(['git', '-C', str(self.root), 'add', '.'], check=True)
        original = release.run
        def run(*args, **kwargs):
            return '/bin/sh\n' if args[0] == 'rustup' else original(*args, **kwargs)
        with patch.object(release, 'run', side_effect=run):
            before = release.fingerprint('windows')
            self.write('README.md', 'documentation')
            self.write('crates/frontend/android/AndroidManifest.xml', 'android only')
            self.assertEqual(before, release.fingerprint('windows'))
            self.write('crates/frontend/src/lib.rs', 'changed source')
            self.assertNotEqual(before, release.fingerprint('windows'))

    def test_version_bump_keeps_packages_and_android_identity_in_step(self):
        for directory, name in [('daemon', 'taud'), ('frontend', 'tau-frontend')]:
            self.write(f'crates/{directory}/Cargo.toml', f'[package]\nname="{name}"\nversion = "0.7.4"\n')
        self.write('Cargo.lock', '[[package]]\nname = "taud"\nversion = "0.7.4"\n[[package]]\nname = "tau-frontend"\nversion = "0.7.4"\n')
        self.write('crates/net/src/lib.rs', 'pub const PROTOCOL_VERSION: u32 = 18;')
        self.write('crates/frontend/android/AndroidManifest.xml', '<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="app.tau.rust" android:versionCode="9" android:versionName="0.7.4-beta"/>')
        release.bump('0.7.5')
        self.assertEqual(('0.7.5', 18, 10), release.metadata())
        release.bump('0.7.5')
        self.assertEqual(('0.7.5', 18, 10), release.metadata())
        with self.assertRaises(AssertionError):
            release.bump('0.7.3')

    def test_windows_verifier_checks_app_archive_and_launcher(self):
        app = self.write('target/x86_64-pc-windows-msvc/release/tau.exe', b'MZapp')
        launcher = self.write('crates/windows/target/x86_64-pc-windows-msvc/release/tau-launcher.exe', b'MZlauncher')
        raw = io.BytesIO()
        with tarfile.open(fileobj=raw, mode='w') as archive:
            member = tarfile.TarInfo('app/Tau Beta.exe')
            member.size = len(app.read_bytes())
            archive.addfile(member, io.BytesIO(app.read_bytes()))
        payload = lzma.compress(raw.getvalue(), format=lzma.FORMAT_ALONE)
        self.write('target/windows-sfx-Tau-Beta-0.7.5/tau-windows-payload.tar.lzma', payload)
        setup = self.write('dist/Tau-Beta-0.7.5-windows-x64.exe', b'MZsetup' + payload + launcher.read_bytes())
        self.assertEqual(setup, release.verify_windows('0.7.5'))
        app.write_bytes(b'MZwrong app')
        with self.assertRaises(AssertionError):
            release.verify_windows('0.7.5')

    def client_release_fixture(self):
        self.write('crates/frontend/Cargo.toml', '[package]\nversion="0.7.5"\n')
        self.write('crates/daemon/Cargo.toml', '[package]\nversion="0.7.5"\n')
        self.write('crates/net/src/lib.rs', 'pub const PROTOCOL_VERSION:u32=18;')
        self.write('crates/frontend/android/AndroidManifest.xml', '<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="app.tau.rust" android:versionCode="10" android:versionName="0.7.5-beta"/>')
        self.write('.gitignore', '/dist/\n/target/\n/cache/\n')
        self.write('scripts/release-clients.sh', (self.old / 'scripts/release-clients.sh').read_text()).chmod(0o755)
        self.write('scripts/build-windows.sh', '#!/bin/sh\necho unexpected-windows-build >&2\nexit 99\n').chmod(0o755)
        self.write('scripts/build-android.sh', '#!/bin/sh\necho unexpected-android-build >&2\nexit 98\n').chmod(0o755)
        self.write('scripts/sender', '#!/bin/sh\necho "$1" >> dist/send-attempts\nexit "${FAIL_SEND:-0}"\n').chmod(0o755)
        helper = self.old / 'scripts/beta-release.py'
        self.write('scripts/beta-release.py', f"""import importlib.util,sys
from pathlib import Path
s=importlib.util.spec_from_file_location('r',{str(helper)!r});r=importlib.util.module_from_spec(s);s.loader.exec_module(r);r.ROOT=Path(__file__).resolve().parent.parent
if sys.argv[1]=='key':print(sys.argv[2]+'-key')
elif sys.argv[1]=='check-key':pass
else:r.main()
""")
        remote = self.root / 'cache/remote.git'
        subprocess.run(['git', 'init', '--bare', '-q', str(remote)], check=True)
        subprocess.run(['git', 'init', '-q', '-b', 'tau2', str(self.root)], check=True)
        subprocess.run(['git', '-C', str(self.root), 'add', '.'], check=True)
        subprocess.run(['git', '-C', str(self.root), '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture'], check=True)
        subprocess.run(['git', '-C', str(self.root), 'remote', 'add', 'origin', str(remote)], check=True)
        subprocess.run(['git', '-C', str(self.root), 'push', '-qu', 'origin', 'tau2'], check=True)
        windows = self.write('dist/Tau-Beta-0.7.5-windows-x64.exe', b'windows')
        android = self.write('dist/Tau-Beta-0.7.5-android-arm64-v8a.apk', b'android')
        sums = self.write('dist/Tau-Beta-0.7.5-SHA256SUMS.txt', b'sums')
        delivery = self.write('dist/releases/0.7.5/delivery.json', b'{}')
        commit = subprocess.check_output(['git', '-C', str(self.root), 'rev-parse', 'HEAD'], text=True).strip()
        release.receipt(self.root / 'dist/releases/0.7.5/clients.json', f'{commit}:windows-key:android-key', [windows, android, sums, delivery])

    def rollout(self, *args, fail=0):
        env = dict(os.environ, XDG_CACHE_HOME=str(self.root / 'cache'), FAIL_SEND=str(fail), PYTHONDONTWRITEBYTECODE='1')
        return subprocess.run([str(self.root / 'scripts/release-clients.sh'), *args], text=True, capture_output=True, env=env, timeout=8)

    def test_completed_client_release_reuses_artifacts_and_never_builds_daemon(self):
        self.client_release_fixture()
        result = self.rollout()
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn('reuse immutable verified release', result.stdout)
        self.assertIn('Daemon build and deployment are separate', self.rollout('--plan').stdout)

    def test_failed_delivery_is_not_automatically_repeated(self):
        self.client_release_fixture()
        args = ['--send-command', str(self.root / 'scripts/sender')]
        self.assertNotEqual(0, self.rollout(*args, fail=23).returncode)
        result = self.rollout(*args)
        self.assertNotEqual(0, result.returncode)
        self.assertIn('Uncertain delivery', result.stderr)
        attempts = self.root / 'dist/send-attempts'
        self.assertEqual(1, len(attempts.read_text().splitlines()))
        result = self.rollout(*args, '--retry-delivery')
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(3, len(attempts.read_text().splitlines()))

    def test_daemon_deployment_escapes_target_service_before_building(self):
        source = (self.old / 'scripts/deploy-daemon.sh').read_text()
        escape = source.index("grep -Fq '/tau2-beta.service'")
        build = source.index('/usr/local/bin/cargo build')
        self.assertLess(escape, build)
        self.assertIn('systemd-run --quiet --scope --collect', source)


if __name__ == '__main__':
    unittest.main()
