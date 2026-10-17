"""Protocol faults with in-memory transports only."""
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import Mock

import northflank_deploy
from northflank_deploy import Halt

ROOT = Path(__file__).parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


native = load('operational_native_deploy', ROOT / 'native-release/deploy.py')
upstream = load('operational_upstream_probe', ROOT / 'fork-upstream-probe.py')


class OperationalProtocols(unittest.TestCase):
    def test_backup_shell_emits_proof_only_after_successful_dump_validation_and_metadata(self):
        scripts = []

        class Captured(Exception):
            pass

        def capture(script):
            scripts.append(script)
            raise Captured()

        adapter = object.__new__(northflank_deploy.Northflank)
        adapter.remote = capture
        with self.assertRaises(Captured):
            adapter.backup(Path('/synthetic-not-created'))
        substitutes = """
test() {
  case "$1" in
    '!') [ "$FAULT" != existing ] ;;
    '-s') [ "$FAULT" != empty ] ;;
    *) command test "$@" ;;
  esac
}
pg_dump() { printf 'dump\\n' >&2; [ "$FAULT" != dump ]; }
pg_restore() { printf 'restore\\n' >&2; [ "$FAULT" != restore ]; }
stat() { [ "$FAULT" != stat ] || return 1; if [ "$FAULT" = size ]; then printf 0; else printf 14; fi; }
sha256sum() { [ "$FAULT" != checksum ] || return 1; printf '%s  synthetic\\n' "$CHECKSUM"; }
"""
        checksum = 'a' * 64
        for fault in ['', 'existing', 'empty', 'dump', 'restore', 'stat', 'size', 'checksum']:
            with self.subTest(fault=fault):
                result = subprocess.run(['sh', '-s'], input=substitutes + scripts[0], text=True,
                                        capture_output=True, env={'PATH': '/usr/bin:/bin', 'FAULT': fault,
                                                                 'CHECKSUM': checksum, 'DATABASE_URL': 'synthetic'})
                if fault:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(result.stdout, '')
                else:
                    self.assertEqual(result.returncode, 0)
                    self.assertEqual(result.stdout, 'PANDA_BACKUP={} 14\n'.format(checksum))
                expected_trace = '' if fault == 'existing' else 'dump\n' if fault == 'dump' else 'dump\nrestore\n'
                self.assertEqual(result.stderr, expected_trace)

    def test_backup_attestation_rejects_crlf_and_non_lf_record_separators(self):
        sha, captured = 'a' * 40, 1700000000
        for suffix in ['\r\n', '\v', '\x85']:
            adapter = Mock()
            with self.subTest(suffix=repr(suffix)):
                with self.assertRaisesRegex(Halt, 'backup-not-attested'):
                    native.deploy(adapter, sha, '101', 'valid-build-1234',
                                  'PANDA_NATIVE_backup_OK {} {}{}'.format(sha, captured, suffix), now=lambda: captured)
                adapter.snapshot.assert_not_called()

    def test_upstream_probe_reads_only_fixed_refs_and_rejects_swapped_identity(self):
        requests = []

        class Client:
            def open(self, request, timeout):
                requests.append((request.full_url, timeout))
                branch = request.full_url.rsplit('/', 1)[1]
                return io.BytesIO(json.dumps({'ref': 'refs/heads/' + branch,
                                             'object': {'sha': 'a' * 40, 'type': 'commit'}}).encode())

        result = upstream.probe(Client())
        self.assertEqual(result, 'UPSTREAM main=' + 'a' * 40 + ' dev=' + 'a' * 40)
        self.assertEqual(requests, [('https://upstream.panda.invalid/git/ref/heads/main', 20),
                                    ('https://upstream.panda.invalid/git/ref/heads/dev', 20)])
        client = Mock()
        client.open.return_value = io.BytesIO(json.dumps({'ref': 'refs/heads/foreign',
                                                        'object': {'sha': 'a' * 40, 'type': 'commit'}}).encode())
        with self.assertRaisesRegex(ValueError, 'invalid ref'):
            upstream.probe(client)
        self.assertEqual(client.open.call_count, 1)


if __name__ == '__main__':
    unittest.main()
