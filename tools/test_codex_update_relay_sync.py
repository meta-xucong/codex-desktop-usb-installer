import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SOURCE = Path(__file__).with_name("codex-update-relay-sync.py")
SPEC = importlib.util.spec_from_file_location("relay_sync", SOURCE)
relay = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(relay)


def asset(name, data, number):
    digest = hashlib.sha256(data).hexdigest()
    return {"id": number, "name": name, "size": len(data), "digest": "sha256:" + digest, "browser_download_url": "https://github.com/Wangnov/codex-app-mirror/releases/download/t/" + name}, data


class RelayTests(unittest.TestCase):
    def setUp(self):
        self.msix, self.msix_data = asset("OpenAI.Codex_x64__x.Msix", b"msix", 11)
        self.manifest, self.manifest_data = asset("release-manifest.json", b'{"version":1}', 12)
        self.sums, self.sums_data = asset("SHA256SUMS-windows.txt", b"0" * 64 + b"  OpenAI.Codex_x64__x.Msix", 13)
        self.release = {"id": 99, "tag_name": "codex-app-test", "published_at": "2026-09-16T00:00:00Z", "assets": [self.msix, self.manifest, self.sums]}

    def test_retry_backoff_and_exhaustion(self):
        calls, sleeps = [], []
        def operation():
            calls.append(1)
            if len(calls) < 3:
                raise relay.RelayError("temporary")
            return "ok"
        cfg = {"retry_attempts": 3, "retry_base_seconds": 2, "retry_max_seconds": 5}
        self.assertEqual(relay.retry_call(operation, "fixture", cfg, sleeps.append), "ok")
        self.assertEqual(sleeps, [2, 4])
        with self.assertRaises(relay.RelayError):
            relay.retry_call(lambda: (_ for _ in ()).throw(relay.RelayError("bad")), "fixture", cfg, lambda _: None)

    def test_concurrent_lock_is_rejected(self):
        class Handle:
            def fileno(self):
                return 7
        fake_fcntl = mock.Mock()
        fake_fcntl.LOCK_EX, fake_fcntl.LOCK_NB = 1, 2
        fake_fcntl.flock.side_effect = BlockingIOError()
        with mock.patch.object(relay, "fcntl", fake_fcntl):
            with self.assertRaises(relay.RelayError):
                relay.acquire_lock(Handle())

    def test_identity_includes_release_and_all_asset_fields(self):
        chosen = (self.msix, self.manifest, self.sums)
        first = relay.identity_from(self.release, *chosen)
        revised = dict(self.msix, id=14, digest="sha256:" + ("a" * 64))
        changed = relay.identity_from(self.release, revised, self.manifest, self.sums)
        self.assertTrue(relay.identity_equal(first, first))
        self.assertFalse(relay.identity_equal(first, changed))

    def test_duplicate_asset_name_and_missing_digest_fail_closed(self):
        with self.assertRaises(relay.RelayError):
            relay.release_assets(dict(self.release, assets=self.release["assets"] + [dict(self.msix)]))
        incomplete = dict(self.manifest)
        incomplete.pop("digest")
        with self.assertRaises(relay.RelayError):
            relay.release_assets(dict(self.release, assets=[self.msix, incomplete, self.sums]))

    def test_manifest_binds_selected_msix(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "release-manifest.json"
            digest = hashlib.sha256(self.msix_data).hexdigest()
            payload = {"codexVersion": "test", "publishedAt": "now", "sources": {"windows": {"architectures": {"x64": {"contentLength": len(self.msix_data)}}}}, "derived": {"latestChecksums": {self.msix["name"]: digest}}}
            path.write_text(json.dumps(payload))
            self.assertEqual(relay.validate_manifest(path, self.msix, digest)["codexVersion"], "test")

    def test_prune_keeps_only_current_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            releases = Path(temporary)
            for name in ("1", "2", "3"):
                (releases / name).mkdir()
            relay.prune_published_releases(releases, "3", 1)
            self.assertEqual(sorted(p.name for p in releases.iterdir()), ["3"])

    def test_journal_path_escape_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            state = root / "state"
            state.mkdir()
            relay.write_json_atomic(state / "publish-journal.json", {"schema": 1, "transactionId": "b" * 32, "phase": "incoming-ready", "incoming": "/tmp/escape", "final": str(root / "export" / "releases" / "2")})
            with self.assertRaises(relay.RelayError):
                relay.recover_publish(root, state / "publish-journal.json")

    def test_publish_revised_same_release_and_recovery(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, export, releases = Path(temporary), Path(temporary) / "export", Path(temporary) / "export" / "releases"
            releases.mkdir(parents=True)
            old = releases / "99"
            old.mkdir()
            old_identity = relay.identity_from(self.release, self.msix, self.manifest, self.sums)
            relay.write_json_atomic(old / "latest.json", dict(old_identity, protocol=2, identity=old_identity))
            pointer = export / "current.pointer"
            fake_switch = lambda _export, target: pointer.write_text(target)
            new_msix = dict(self.msix, id=14)
            new_assets = (new_msix, self.manifest, self.sums)
            new_identity = relay.identity_from(self.release, *new_assets)
            stage = root / "work" / "stage"
            stage.mkdir(parents=True)
            (stage / new_msix["name"]).write_bytes(self.msix_data)
            (stage / self.manifest["name"]).write_bytes(self.manifest_data)
            (stage / self.sums["name"]).write_bytes(self.sums_data)
            cfg = {"root": root, "repository": "Wangnov/codex-app-mirror", "architecture": "x64", "max_published_releases": 2}
            digests = {key: value["sha256"] for key, value in new_identity["assets"].items()}
            with mock.patch.object(relay, "switch_current", fake_switch):
                relay.publish(self.release, cfg, new_assets, stage, digests)
            self.assertEqual(pointer.read_text().replace("\\", "/"), "releases/99")
            self.assertEqual(json.loads((releases / "99" / "latest.json").read_text())["identity"]["assets"]["windows-msix"]["id"], 14)
            self.assertFalse((root / "state" / "publish-journal.json").exists())

    def test_journal_recovery_finishes_incoming(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, export, releases, state = Path(temporary), Path(temporary) / "export", Path(temporary) / "export" / "releases", Path(temporary) / "state"
            releases.mkdir(parents=True)
            (releases / "1").mkdir()
            pointer = export / "current.pointer"
            fake_switch = lambda _export, target: pointer.write_text(target)
            incoming, final, retired = releases / ".incoming-2-t", releases / "2", releases / ".retired-2-t"
            incoming.mkdir()
            (incoming / "latest.json").write_text("{}")
            relay.write_json_atomic(state / "publish-journal.json", {"schema": 1, "transactionId": "a" * 32, "phase": "incoming-ready", "incoming": str(incoming), "final": str(final), "retired": str(retired)})
            with mock.patch.object(relay, "switch_current", fake_switch):
                relay.recover_publish(root, state / "publish-journal.json")
            self.assertEqual(pointer.read_text().replace("\\", "/"), "releases/2")
            self.assertTrue(final.exists())
            self.assertFalse((state / "publish-journal.json").exists())


if __name__ == "__main__":
    unittest.main()
