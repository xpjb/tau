#!/usr/bin/env python3
"""Fast isolated automation tests. No Cargo, network, SDK builds or services."""
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

spec = importlib.util.spec_from_file_location('release', Path(__file__).with_name('beta-release.py'))
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
        p = self.root / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data.encode() if isinstance(data, str) else data)
        return p

    def test_receipts_reuse_only_exact_inputs_and_unchanged_artifacts(self):
        body = self.write('app.exe', b'MZoriginal')
        record = self.root / 'receipt.json'
        release.receipt(record, 'inputs', [body])
        self.assertTrue(release.hit(record, 'inputs'))
        self.assertFalse(release.hit(record, 'other-inputs'))
        body.write_bytes(b'MZmodified')
        self.assertFalse(release.hit(record, 'inputs'))
        body.unlink()
        self.assertFalse(release.hit(record, 'inputs'))
        record.write_text('{partial')
        self.assertFalse(release.hit(record, 'inputs'))

    def test_docs_and_frontend_changes_do_not_invalidate_daemon(self):
        for name in ['Cargo.toml', 'Cargo.lock', 'daemon/src/lib.rs', 'frontend/src/lib.rs', 'daemon/README.md']:
            self.write(name, 'initial')
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)
        subprocess.run(['git', '-C', str(self.root), 'add', '.'], check=True)
        original = release.run
        def run(*args, **kwargs):
            return '/bin/sh\n' if args[0] == 'rustup' else original(*args, **kwargs)
        with patch.object(release, 'run', side_effect=run):
            before = release.fingerprint('daemon')
            self.write('daemon/README.md', 'documentation')
            self.write('frontend/src/lib.rs', 'frontend only')
            self.assertEqual(before, release.fingerprint('daemon'))
            self.write('daemon/src/lib.rs', 'changed source')
            self.assertNotEqual(before, release.fingerprint('daemon'))

    def test_version_bump_keeps_all_packages_in_step_without_rebumping(self):
        for directory, name in [('daemon', 'taud'), ('frontend', 'tau-frontend')]:
            self.write(directory + '/Cargo.toml', f'[package]\nname="{name}"\nversion = "0.7.4"\n')
        self.write('Cargo.lock', '[[package]]\nname = "taud"\nversion = "0.7.4"\n[[package]]\nname = "tau-frontend"\nversion = "0.7.4"\n')
        self.write('protocol/src/lib.rs', 'pub const PROTOCOL_VERSION: u32 = 18;')
        self.write('frontend/android/AndroidManifest.xml', '<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="app.tau.rust" android:versionCode="9" android:versionName="0.7.4-beta"/>')
        release.bump('0.7.5')
        self.assertEqual(('0.7.5', 18, 10), release.metadata())
        release.bump('0.7.5')
        self.assertEqual(('0.7.5', 18, 10), release.metadata())
        with self.assertRaises(AssertionError):
            release.bump('0.7.3')

    def test_installer_verifies_both_app_archive_and_embedded_launcher(self):
        app = self.write('target/x86_64-pc-windows-msvc/release/tau.exe', b'MZapp')
        launcher = self.write('windows/target/x86_64-pc-windows-msvc/release/tau-launcher.exe', b'MZlauncher')
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

    def test_cancellation_stops_only_our_managed_envelope(self):
        def run(*args, **kwargs):
            if args[1] == 'list-units':
                return 'run-owned.service loaded active running\nrun-other.service loaded active running\n'
            return '100\n' if args[2] == 'run-owned.service' else '200\n'
        def environ(path):
            return b'TAU_BETA_RELEASE_RUN=ours\0' if str(path) == '/proc/100/environ' else b'TAU_BETA_RELEASE_RUN=another-job\0'
        with patch.object(release, 'run', side_effect=run), patch.object(Path, 'read_bytes', environ), patch.object(release.subprocess, 'run') as stop:
            release.stop_owned('ours')
            stop.assert_called_once_with(['systemctl', 'stop', 'run-owned.service'], check=True)

    def cached_rollout_fixture(self):
        # Exercise the real orchestration with immutable fake artifacts. SDK,
        # keychain and Cargo commands are replaced by explicit no-work doubles.
        self.write('frontend/Cargo.toml', '[package]\nversion="0.7.5"\n')
        self.write('daemon/Cargo.toml', '[package]\nversion="0.7.5"\n')
        self.write('protocol/src/lib.rs', 'pub const PROTOCOL_VERSION:u32=18;')
        self.write('frontend/android/AndroidManifest.xml', '<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="app.tau.rust" android:versionCode="10" android:versionName="0.7.5-beta"/>')
        self.write('.gitignore', '/dist/\n/target/\n/cache/\n')
        source = (self.old / 'scripts/release-beta.sh').read_text()
        source = source.replace('cargo=/usr/local/bin/cargo', 'cargo="$root/scripts/forbidden-build"')
        self.write('scripts/release-beta.sh', source).chmod(0o755)
        self.write('scripts/forbidden-build', '#!/bin/sh\necho unexpected-build >&2\nexit 99\n').chmod(0o755)
        self.write('scripts/sender', '#!/bin/sh\necho "$1" >> dist/send-attempts\nexit "${FAIL_SEND:-0}"\n').chmod(0o755)
        helper = self.old / 'scripts/beta-release.py'
        self.write('scripts/beta-release.py', f"""import importlib.util,sys
from pathlib import Path
s=importlib.util.spec_from_file_location('r',{str(helper)!r});r=importlib.util.module_from_spec(s);s.loader.exec_module(r);r.ROOT=Path(__file__).resolve().parent.parent
if sys.argv[1]=='key':print(sys.argv[2]+'-key')
elif sys.argv[1] in ('check-key','stop-owned'):pass
else:r.main()
""")
        subprocess.run(['git', 'init', '-q', '-b', 'tau2-integration', str(self.root)], check=True)
        subprocess.run(['git', '-C', str(self.root), 'add', '.'], check=True)
        subprocess.run(['git', '-C', str(self.root), '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture'], check=True)
        files = [self.write('dist/Tau-Beta-0.7.5-' + suffix, b'immutable') for suffix in ['taud', 'windows-x64.exe', 'android-arm64-v8a.apk']]
        release.receipt(self.root / 'dist/releases/0.7.5/complete.json', 'daemon-key:windows-key:android-key', files)

    def rollout(self, *args, fail=0):
        env = dict(os.environ, XDG_CACHE_HOME=str(self.root / 'cache'), FAIL_SEND=str(fail), PYTHONDONTWRITEBYTECODE='1')
        return subprocess.run([str(self.root / 'scripts/release-beta.sh'), *args], text=True, capture_output=True, env=env, timeout=8)

    def test_completed_release_resumes_without_builds_tests_or_services(self):
        self.cached_rollout_fixture()
        result = self.rollout()
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn('reuse immutable verified release', result.stdout)
        self.assertIn('pushed=false deployed=false', result.stdout)

    def test_failed_delivery_is_not_automatically_repeated(self):
        self.cached_rollout_fixture()
        args = ['--send-command', str(self.root / 'scripts/sender')]
        self.assertNotEqual(0, self.rollout(*args, fail=23).returncode)
        result = self.rollout(*args)
        self.assertNotEqual(0, result.returncode)
        self.assertIn('Uncertain delivery', result.stderr)
        attempts = self.root / 'dist/send-attempts'
        self.assertEqual(1, len(attempts.read_text().splitlines()))
        result = self.rollout(*args, '--retry-delivery')
        self.assertEqual(0, result.returncode, result.stderr)
        lines = attempts.read_text().splitlines()
        self.assertEqual(3, len(lines))
        self.assertTrue(lines[-2].endswith('windows-x64.exe'))
        self.assertTrue(lines[-1].endswith('android-arm64-v8a.apk'))
        self.assertEqual(0, self.rollout(*args).returncode)
        self.assertEqual(3, len(attempts.read_text().splitlines()))


if __name__ == '__main__':
    unittest.main()
