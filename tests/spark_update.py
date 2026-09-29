"""Signed workstation updates, exercised through the operator entrypoint."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/update-spark.py"

CLIENT = '''#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
state = pathlib.Path(os.environ["UPDATE_FIXTURE"])
with (state / "calls.jsonl").open("a") as out:
    out.write(json.dumps(args) + "\\n")
command = args[1]
changed = (state / "applied").exists()
failure = os.environ.get("UPDATE_FAILURE")
if command == "status":
    result = {"schema":"sparkplane.status/v1", "agent":"0.1.6",
              "executor":"0.1.6", "read_only":False, "degraded_reasons":[]}
    if changed and failure == "health": result["read_only"] = True
    if changed and failure == "version": result["executor"] = "old-version"
elif command == "ps":
    instance = {"id":"i_fixture", "name":"model", "model_id":"m_fixture",
                "model":"immutable", "model_commit":"commit", "engine_id":"engine",
                "engine_fingerprint":"engine-sha", "artifact_fingerprint":"artifact-sha",
                "artifacts":{}, "resources":{}, "context_window":262144, "generation":10,
                "desired":"running", "observed":"healthy", "healthy":True}
    if changed and failure == "generation": instance["generation"] += 1
    if changed and failure == "context": instance["context_window"] = 32768
    if changed and failure == "engine": instance["engine_fingerprint"] = "changed"
    result = {"schema":"sparkplane.instance-list/v1", "instances":[instance]}
elif command == "upgrade":
    applied = "--yes" in args
    if not applied and failure == "preview": sys.exit(3)
    probe = pathlib.Path(args[args.index("--probe") + 1])
    import hashlib
    digest = hashlib.sha256(probe.read_bytes()).hexdigest()
    result = {"schema":"sparkplane.install-manifest/v1", "operation":"upgrade",
              "host_alias":args[0], "dry_run":not applied,
              "inventory":{"existing_installation":{"present":True}},
              "probe":{"local_sha256":digest, "reported_sha256":digest, "removed":True},
              "protected_before":{"sha256":"protected"},
              "execution":{"state":"applied" if applied else "planned"}}
    if applied: (state / "applied").touch()
elif command == "doctor": result = {"schema":"sparkplane.doctor/v1", "checks":[]}
elif command == "launch": result = {"schema":"sparkplane.launch/v1"}
else: sys.exit(2)
print(json.dumps(result))
'''


class UpdateWorkflow(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        checkout = self.root / "checkout/scripts"
        checkout.mkdir(parents=True)
        self.script = checkout / "update-spark.py"
        shutil.copyfile(SCRIPT, self.script)
        self.release = self.root / "release"
        self.release.mkdir()
        self.home = self.root / "home"
        self.home.mkdir()
        self.tools = self.root / "tools"
        self.tools.mkdir()
        self.secret = self.root / "release.key"
        self.public = self.root / "release.pub"
        subprocess.run(["minisign", "-G", "-W", "-s", str(self.secret), "-p", str(self.public)],
                       check=True, capture_output=True)
        ssh = self.tools / "ssh"
        ssh.write_text('#!/usr/bin/env python3\nimport os,pathlib\nprint(pathlib.Path(os.environ["UPDATE_AUTHORITY"]).read_text(),end="")\n')
        ssh.chmod(0o755)
        self.env = dict(os.environ, HOME=str(self.home),
                        XDG_CONFIG_HOME=str(self.home / ".config"),
                        XDG_DATA_HOME=str(self.home / ".local/share"),
                        UPDATE_FIXTURE=str(self.root), UPDATE_AUTHORITY=str(self.public),
                        PATH=f"{self.tools}:{os.environ['PATH']}")
        for key in list(self.env):
            if key.startswith("SPARKPLANE_") or key.startswith("UPDATE_FAILURE"):
                del self.env[key]
        client = self.release / "sparkplane-client"
        client.write_text(CLIENT)
        client.chmod(0o555)
        appliance = self.release / "appliance"
        engines = appliance / "configs/sparkplane/engines"
        engines.mkdir(parents=True)
        (appliance / "sparkplane-aarch64").write_bytes(b"arm64 appliance")
        (appliance / "configs/sparkplane/models.toml").write_text("models = []\n")
        (engines / "engine.toml").write_text("engine = 'fixture'\n")
        self.manifest(appliance, ["sparkplane-aarch64", "configs/sparkplane/models.toml",
                                  "configs/sparkplane/engines/engine.toml"])
        self.sign(appliance / "SHA256SUMS")
        shutil.copyfile(self.public, self.release / "minisign.pub")
        (self.release / "update.json").write_text(json.dumps({
            "schema":"sparkplane.workstation-release/v1", "version":"0.1.6",
            "machine":os.uname().machine, "checks":["lint", "test", "test-client", "audit"],
        }))
        self.seal()

    def manifest(self, directory, files):
        (directory / "SHA256SUMS").write_text("".join(
            f"{hashlib.sha256((directory / name).read_bytes()).hexdigest()}  {name}\n"
            for name in files))

    def sign(self, path):
        subprocess.run(["minisign", "-S", "-s", str(self.secret), "-m", str(path)],
                       check=True, capture_output=True)

    def seal(self):
        self.manifest(self.release, ["sparkplane-client", "update.json", "minisign.pub",
                                    "appliance/SHA256SUMS", "appliance/SHA256SUMS.minisig"])
        self.sign(self.release / "SHA256SUMS")

    def run_update(self, *args, failure=None):
        env = dict(self.env)
        if failure:
            env["UPDATE_FAILURE"] = failure
        output = subprocess.run([
            "python3", str(self.script), "fixture-spark", "--prepared", str(self.release),
            "--public-key", str(self.public), "--evidence-dir", str(self.root / "evidence"),
            "--json", *args], env=env, capture_output=True, text=True)
        result = json.loads(output.stdout)
        self.assertNotIn("Traceback", output.stderr)
        return output, result

    def calls(self):
        path = self.root / "calls.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def test_preview_never_activates_or_installs_client(self):
        output, result = self.run_update()
        self.assertEqual(output.returncode, 0, output.stderr)
        self.assertEqual(result["state"], "previewed")
        self.assertFalse((self.root / "applied").exists())
        self.assertFalse((self.home / ".local/bin/sparkplane").exists())
        calls = self.calls()
        self.assertTrue(any("--dry-run" in call for call in calls))
        self.assertFalse(any("--yes" in call for call in calls))
        for call in calls:
            if call[1] == "upgrade":
                self.assertEqual(call[call.index("--probe") + 1],
                                 str(self.release / "appliance/sparkplane-aarch64"))
        evidence = Path(result["evidence"])
        self.assertEqual(evidence.stat().st_mode & 0o777, 0o700)
        self.assertEqual((evidence / "upgrade-dry-run.json").stat().st_mode & 0o777, 0o600)

    def test_apply_preserves_engine_and_publishes_verified_native_client(self):
        output, result = self.run_update("--apply", "--codex-model", "model:tag")
        self.assertEqual(output.returncode, 0, output.stderr)
        self.assertEqual(result["state"], "applied")
        calls = self.calls()
        commands = [call[1] for call in calls]
        self.assertFalse(set(commands) & {"stop", "serve", "download"})
        upgrades = [call for call in calls if call[1] == "upgrade"]
        self.assertEqual(len(upgrades), 2)
        self.assertIn("--dry-run", upgrades[0])
        self.assertIn("--yes", upgrades[1])
        self.assertIn("--release-public-key", upgrades[1])
        link = self.home / ".local/bin/sparkplane"
        self.assertTrue(link.is_symlink())
        self.assertEqual(link.read_bytes(), (self.release / "sparkplane-client").read_bytes())
        self.assertEqual(link.stat().st_mode & 0o777, 0o555)
        self.assertIn("launch", commands)
        self.assertIn("--config", calls[-1])

    def test_corrupt_client_is_rejected_before_it_can_execute(self):
        (self.release / "sparkplane-client").chmod(0o755)
        (self.release / "sparkplane-client").write_text(CLIENT + "\n# corruption\n")
        output, result = self.run_update("--apply")
        self.assertNotEqual(output.returncode, 0)
        self.assertEqual(result["state"], "failed")
        self.assertEqual(self.calls(), [])

    def test_corrupt_appliance_is_rejected_before_activation(self):
        (self.release / "appliance/sparkplane-aarch64").write_bytes(b"corruption")
        output, _ = self.run_update("--apply")
        self.assertNotEqual(output.returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_signature_cannot_be_replaced_with_bundle_authority(self):
        other_key, other_pub = self.root / "other.key", self.root / "other.pub"
        subprocess.run(["minisign", "-G", "-W", "-s", str(other_key), "-p", str(other_pub)],
                       check=True, capture_output=True)
        shutil.copyfile(other_pub, self.release / "minisign.pub")
        self.secret = other_key
        self.seal()
        output, _ = self.run_update("--apply")
        self.assertNotEqual(output.returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_installed_authority_mismatch_rejects_even_a_locally_signed_release(self):
        foreign = self.root / "foreign.pub"
        foreign.write_text("untrusted comment: minisign public key\n" + "A" * 56 + "\n")
        self.env["UPDATE_AUTHORITY"] = str(foreign)
        output, _ = self.run_update("--apply")
        self.assertNotEqual(output.returncode, 0)
        self.assertFalse((self.root / "applied").exists())

    def test_failed_preview_prevents_activation(self):
        output, _ = self.run_update("--apply", failure="preview")
        self.assertNotEqual(output.returncode, 0)
        self.assertFalse((self.root / "applied").exists())
        self.assertFalse(any("--yes" in call for call in self.calls()))

    def test_failed_health_or_changed_engine_prevents_client_publication(self):
        for failure in ["health", "context", "generation", "engine", "version"]:
            with self.subTest(failure=failure):
                (self.root / "applied").unlink(missing_ok=True)
                output, _ = self.run_update("--apply", failure=failure)
                self.assertNotEqual(output.returncode, 0)
                self.assertFalse((self.home / ".local/bin/sparkplane").exists())

    def test_symlink_in_signed_inventory_is_rejected(self):
        payload = self.release / "appliance/sparkplane-aarch64"
        outside = self.root / "outside"
        payload.rename(outside)
        payload.symlink_to(outside)
        output, _ = self.run_update("--apply")
        self.assertNotEqual(output.returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_wrong_native_architecture_is_rejected_without_execution(self):
        metadata = self.release / "update.json"
        data = json.loads(metadata.read_text())
        data["machine"] = "unsupported"
        metadata.write_text(json.dumps(data))
        self.seal()
        output, _ = self.run_update("--apply")
        self.assertNotEqual(output.returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_no_install_client_leaves_existing_executable_unchanged(self):
        link = self.home / ".local/bin/sparkplane"
        link.parent.mkdir(parents=True)
        link.write_text("unmanaged client")
        output, _ = self.run_update("--apply", "--no-install-client")
        self.assertEqual(output.returncode, 0, output.stderr)
        self.assertEqual(link.read_text(), "unmanaged client")

    def test_encrypted_credential_signs_without_exposing_password(self):
        password = b"fixture-only signing password"
        key = self.root / "encrypted.key"
        public = self.root / "encrypted.pub"
        subprocess.run(["minisign", "-G", "-s", str(key), "-p", str(public)],
                       input=password + b"\n" + password + b"\n", check=True, capture_output=True)
        key.chmod(0o600)
        credential = self.root / "password.cred"
        credential.write_bytes(b"encrypted fixture credential")
        credential.chmod(0o600)
        decrypt = self.tools / "systemd-creds"
        decrypt.write_text('#!/usr/bin/env python3\nimport sys\n'
                           'assert sys.argv[1:4] == ["--user", "--name=sparkplane-release-password", "decrypt"]\n'
                           'sys.stdout.write("fixture-only signing password")\n')
        decrypt.chmod(0o755)
        manifest = self.root / "to-sign"
        manifest.write_text("fixture manifest\n")
        code = '''import importlib.util, pathlib, sys
spec = importlib.util.spec_from_file_location("updater", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
module.sign_manifest(*map(pathlib.Path, sys.argv[2:]))
'''
        output = subprocess.run(["python3", "-c", code, str(SCRIPT), str(manifest), str(key),
                                 str(credential)], env=self.env, capture_output=True)
        self.assertEqual(output.returncode, 0, output.stderr)
        self.assertNotIn(password, output.stdout + output.stderr)
        subprocess.run(["minisign", "-V", "-m", str(manifest), "-p", str(public)],
                       check=True, capture_output=True)

    def test_path_traversal_in_signed_inventory_is_rejected(self):
        manifest = self.release / "appliance/SHA256SUMS"
        manifest.write_text(manifest.read_text() + "0" * 64 + "  ../../outside\n")
        self.sign(manifest)
        self.seal()
        output, _ = self.run_update("--apply")
        self.assertNotEqual(output.returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_conflicting_local_executable_prevents_remote_apply(self):
        link = self.home / ".local/bin/sparkplane"
        link.parent.mkdir(parents=True)
        link.write_text("unmanaged client")
        output, _ = self.run_update("--apply")
        self.assertNotEqual(output.returncode, 0)
        self.assertFalse((self.root / "applied").exists())
        self.assertEqual(link.read_text(), "unmanaged client")


if __name__ == "__main__":
    unittest.main()
