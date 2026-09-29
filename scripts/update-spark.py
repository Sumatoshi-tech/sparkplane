#!/usr/bin/env python3
"""Build, sign, preview and apply a workstation's Sparkplane update."""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
CHECKS = ["lint", "test", "test-client", "audit"]
OUTER_FILES = {"sparkplane-client", "update.json", "minisign.pub",
               "appliance/SHA256SUMS", "appliance/SHA256SUMS.minisig"}
INSTANCE_FIELDS = ["id", "name", "model_id", "model", "model_commit", "engine_id",
                   "engine_fingerprint", "artifact_fingerprint", "artifacts", "resources",
                   "context_window", "generation", "desired"]


class UpdateError(Exception):
    pass


def progress(message):
    print(f"spark-update: {message}", file=sys.stderr, flush=True)


def digest(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def regular(path):
    if not stat.S_ISREG(path.lstat().st_mode):
        raise UpdateError(f"expected a regular file: {path}")


def private_file(path):
    regular(path)
    info = path.stat()
    if info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise UpdateError(f"signing secret must be owned by this user with mode 0600: {path}")


def run(argv, *, cwd=ROOT, output=None, input_bytes=None, timeout=1800):
    """Inherit authentication stdin; keep all subprocess output off JSON stdout."""
    with output.open("xb") if output else open(os.devnull, "wb") as handle:
        result = subprocess.run(argv, cwd=cwd, input=input_bytes,
                                stdout=handle if output else sys.stderr,
                                stderr=sys.stderr, timeout=timeout)
    if result.returncode:
        raise UpdateError(f"{Path(argv[0]).name} failed (exit {result.returncode}); see stderr")


def write_json(path, value):
    with path.open("x", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2)
        handle.write("\n")


def manifest_entries(root):
    entries = {}
    for line in (root / "SHA256SUMS").read_text().splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9_./-]+)", line)
        if not match:
            raise UpdateError("checksum inventory has an invalid entry")
        sha, name = match.groups()
        path = Path(name)
        if path.is_absolute() or ".." in path.parts or name in entries:
            raise UpdateError("checksum inventory has an escaping or duplicate path")
        candidate = root / path
        if not candidate.resolve(strict=True).is_relative_to(root.resolve(strict=True)):
            raise UpdateError("checksum inventory escapes the release")
        # Reject symlinks, including directory components within the bundle.
        for parent in [candidate, *candidate.parents]:
            if parent == root:
                break
            if parent.is_symlink():
                raise UpdateError("checksum inventory contains a symlink")
        regular(candidate)
        if digest(candidate) != sha:
            raise UpdateError(f"checksum mismatch: {name}")
        entries[name] = sha
    if not entries:
        raise UpdateError("checksum inventory is empty")
    return entries


def public_identity(value):
    lines = [line.strip() for line in value.splitlines() if line.strip()]
    if len(lines) == 2 and lines[0].startswith("untrusted comment: minisign public key"):
        lines = lines[1:]
    if len(lines) != 1 or not re.fullmatch(r"[A-Za-z0-9+/=]{56}", lines[0]):
        raise UpdateError("invalid release public key")
    return lines[0]


def verify_signature(manifest, public_key):
    regular(manifest)
    regular(Path(str(manifest) + ".minisig"))
    run(["minisign", "-V", "-m", str(manifest), "-p", str(public_key)])


def verify_release(release, public_key):
    # Verify the external authority before executing any bundled client.
    verify_signature(release / "SHA256SUMS", public_key)
    if set(manifest_entries(release)) != OUTER_FILES:
        raise UpdateError("workstation release has an unexpected signed inventory")
    if public_identity((release / "minisign.pub").read_text()) != public_identity(public_key.read_text()):
        raise UpdateError("bundled public key differs from the pinned authority")
    appliance = release / "appliance"
    verify_signature(appliance / "SHA256SUMS", public_key)
    entries = manifest_entries(appliance)
    engine_names = {f"configs/sparkplane/engines/{path.name}"
                    for path in (appliance / "configs/sparkplane/engines").glob("*.toml")}
    if not engine_names or set(entries) != {"sparkplane-aarch64", "configs/sparkplane/models.toml", *engine_names}:
        raise UpdateError("appliance release has an unexpected signed inventory")
    metadata = json.loads((release / "update.json").read_text())
    if (metadata.get("schema") != "sparkplane.workstation-release/v1"
            or metadata.get("machine") != platform.machine()
            or metadata.get("checks") != CHECKS
            or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?", metadata.get("version", ""))):
        raise UpdateError("release metadata, native architecture or required checks do not match")
    return metadata


def installed_authority(host, public_key, evidence):
    # This fixed read needs no sudo. OpenSSH supplies transport authentication.
    path = evidence / "installed-minisign.pub"
    run(["ssh", host, "cat /opt/sparkplane/current/minisign.pub"], output=path, timeout=120)
    if public_identity(path.read_text()) != public_identity(public_key.read_text()):
        raise UpdateError("installed release authority differs from the pinned key; refusing to replace it")


def sign_manifest(manifest, key, credential):
    private_file(key)
    password = None
    if credential:
        private_file(credential)
        result = subprocess.run([
            "systemd-creds", "--user", "--name=sparkplane-release-password",
            "decrypt", str(credential), "-"], capture_output=True, timeout=30)
        if result.returncode or not result.stdout or len(result.stdout) > 65536:
            raise UpdateError("cannot decrypt the workstation signing credential")
        # Plaintext remains in memory and Minisign's stdin, never argv or logs.
        password = result.stdout.rstrip(b"\n") + b"\n"
    elif not sys.stdin.isatty():
        raise UpdateError("signing needs a terminal or the encrypted password.cred credential")
    run(["minisign", "-S", "-m", str(manifest), "-s", str(key)], input_bytes=password)


def write_manifest(root, names):
    with (root / "SHA256SUMS").open("x", encoding="utf-8") as handle:
        for name in sorted(names):
            handle.write(f"{digest(root / name)}  {name}\n")


def prepare(args, public_key, evidence):
    signing = args.signing_dir
    key = signing / "release.key"
    credential = signing / "password.cred"
    credential = credential if credential.exists() else None
    private_file(key)
    installed_authority(args.host, public_key, evidence)
    for check in CHECKS:
        progress(f"running make {check}")
        log = evidence / f"check-{check}.log"
        with log.open("xb") as handle:
            result = subprocess.run(["make", check], cwd=ROOT, stdout=handle,
                                    stderr=subprocess.STDOUT, timeout=1800)
        if result.returncode:
            raise UpdateError(f"make {check} failed; inspect {log}")
    release = Path(tempfile.mkdtemp(prefix="release-", dir=ROOT / "target/spark-updates"))
    progress(f"building native client and ARM64 appliance into {release}")
    target = ROOT / "target"
    run(["cargo", "auditable", "build", "--locked", "--release", "--no-default-features",
         "--bin", "sparkplane", "--target-dir", str(target)])
    shutil.copyfile(target / "release/sparkplane", release / "sparkplane-client")
    (release / "sparkplane-client").chmod(0o555)
    run(["cargo", "auditable", "zigbuild", "--locked", "--release",
         "--target", "aarch64-unknown-linux-gnu", "--no-default-features", "--features",
         "appliance", "--bin", "sparkplane", "--target-dir", str(target)])
    run(["bash", str(ROOT / "scripts/package-spark-release.sh"),
         str(target / "aarch64-unknown-linux-gnu/release/sparkplane"), str(release / "appliance")])
    manifest_entries(release / "appliance")
    sign_manifest(release / "appliance/SHA256SUMS", key, credential)
    shutil.copyfile(public_key, release / "minisign.pub")
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())
    revision = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT,
                              check=True, capture_output=True, text=True).stdout.strip()
    dirty = subprocess.run(["git", "status", "--porcelain"], cwd=ROOT,
                           check=True, capture_output=True).stdout != b""
    write_json(release / "update.json", {
        "schema":"sparkplane.workstation-release/v1", "version":cargo["workspace"]["package"]["version"],
        "machine":platform.machine(), "checks":CHECKS, "revision":revision, "dirty":dirty,
    })
    write_manifest(release, OUTER_FILES)
    sign_manifest(release / "SHA256SUMS", key, credential)
    return release


def client_json(release, args, evidence, name, command):
    output = evidence / f"{name}.json"
    argv = [str(release / "sparkplane-client"), args.host, *command, "--json"]
    if args.config_dir:
        argv.extend(["--config-dir", str(args.config_dir)])
    progress(name)
    run(argv, output=output)
    value = json.loads(output.read_text())
    if not isinstance(value, dict):
        raise UpdateError(f"{name} returned an invalid document")
    return value


def healthy_status(status, version=None):
    if (status.get("schema") != "sparkplane.status/v1" or status.get("read_only") is not False
            or status.get("degraded_reasons") != []):
        raise UpdateError("control plane is not healthy and writable")
    if version and (status.get("agent") != version or status.get("executor") != version):
        raise UpdateError("agent and executor versions differ from the signed candidate")


def preserve_instances(before, after):
    if not isinstance(before.get("instances"), list) or not isinstance(after.get("instances"), list):
        raise UpdateError("instance inventory is invalid")
    observed = {instance["id"]: instance for instance in after["instances"]}
    for instance in before["instances"]:
        if instance.get("desired") != "running":
            continue
        current = observed.get(instance["id"], {})
        if (any(instance.get(field) != current.get(field) for field in INSTANCE_FIELDS)
                or (instance.get("healthy") is True and current.get("healthy") is not True)):
            raise UpdateError(f"running instance changed during the update: {instance['name']}")


def client_destination():
    data = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share"))
    return data / "sparkplane/client/releases", Path.home() / ".local/bin/sparkplane"


def check_client_destination():
    _, link = client_destination()
    if link.exists() and not link.is_symlink():
        raise UpdateError(f"local client is not a managed symlink: {link}; use --no-install-client")


def publish_client(release, public_key):
    # Retain the prior target and signed evidence; do not change the release updater's `current`.
    releases, link = client_destination()
    releases.mkdir(parents=True, exist_ok=True)
    destination = releases / f"workstation-{digest(release / 'SHA256SUMS')}"
    if destination.exists():
        verify_signature(destination / "SHA256SUMS", public_key)
        if (set(manifest_entries(destination)) != OUTER_FILES
                or digest(destination / "SHA256SUMS") != digest(release / "SHA256SUMS")):
            raise UpdateError("existing content-addressed client release differs from this update")
    else:
        staged = Path(tempfile.mkdtemp(prefix=".update-", dir=releases))
        try:
            for name in [*OUTER_FILES, "SHA256SUMS", "SHA256SUMS.minisig"]:
                target = staged / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(release / name, target)
                target.chmod(0o555 if name == "sparkplane-client" else 0o444)
            staged.rename(destination)
        finally:
            if staged.exists():
                shutil.rmtree(staged)
    check_client_destination()
    link.parent.mkdir(parents=True, exist_ok=True)
    staged_link = link.parent / f".sparkplane-update-{os.getpid()}"
    try:
        staged_link.symlink_to(destination / "sparkplane-client")
        staged_link.replace(link)
    finally:
        staged_link.unlink(missing_ok=True)
    return str(link)


def update(args, evidence, result):
    public_key = (args.public_key or args.signing_dir / "release.pub").resolve(strict=True)
    regular(public_key)
    if public_key.stat().st_mode & 0o022:
        raise UpdateError("pinned public key is writable by another user")
    if args.prepared:
        release = args.prepared.resolve(strict=True)
    else:
        release = prepare(args, public_key, evidence)
    result["release"] = str(release)
    metadata = verify_release(release, public_key)
    if args.prepared:
        installed_authority(args.host, public_key, evidence)
    before_status = client_json(release, args, evidence, "status-before", ["status"])
    healthy_status(before_status)
    before = client_json(release, args, evidence, "instances-before", ["ps"])
    command = ["upgrade", "--probe", str(release / "appliance/sparkplane-aarch64"),
               "--release-manifest", str(release / "appliance/SHA256SUMS")]
    plan = client_json(release, args, evidence, "upgrade-dry-run", [*command, "--dry-run"])
    sha = digest(release / "appliance/sparkplane-aarch64")
    if (plan.get("schema") != "sparkplane.install-manifest/v1"
            or plan.get("host_alias") != args.host or plan.get("operation") != "upgrade"
            or plan.get("dry_run") is not True
            or plan.get("inventory", {}).get("existing_installation", {}).get("present") is not True
            or plan.get("probe", {}).get("local_sha256") != sha
            or plan.get("probe", {}).get("reported_sha256") != sha
            or plan.get("probe", {}).get("removed") is not True):
        raise UpdateError("dry-run did not inspect and clean up the exact candidate")
    result["state"] = "previewed"
    if not args.apply:
        return
    if not args.no_install_client:
        check_client_destination()
    # Recheck signed bytes immediately before the managed activation.
    verify_release(release, public_key)
    result["state"] = "applying"
    applied = client_json(release, args, evidence, "upgrade", [*command, "--yes",
                          "--release-signature", str(release / "appliance/SHA256SUMS.minisig"),
                          "--release-public-key", str(public_key)])
    if applied.get("execution", {}).get("state") != "applied":
        raise UpdateError("upgrade did not report an applied release")
    result["appliance_applied"] = True
    if applied.get("protected_before") != plan.get("protected_before"):
        raise UpdateError("protected DGX fingerprint changed between preview and activation")
    after_status = client_json(release, args, evidence, "status-after", ["status"])
    healthy_status(after_status, metadata["version"])
    after = client_json(release, args, evidence, "instances-after", ["ps"])
    preserve_instances(before, after)
    client_json(release, args, evidence, "doctor-after", ["doctor"])
    if not args.no_install_client:
        result["client"] = publish_client(release, public_key)
    if args.codex_model:
        client_json(release, args, evidence, "codex-config", [
            "launch", "codex", "--model", args.codex_model, "--config"])
    result["state"] = "applied"


def main():
    config = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("host", help="existing OpenSSH/Sparkplane host alias")
    parser.add_argument("--apply", action="store_true", help="apply after a successful preview")
    parser.add_argument("--prepared", type=Path, help="reuse a signed release; no rebuild or signing")
    parser.add_argument("--signing-dir", type=Path, default=config / "sparkplane-release-signing")
    parser.add_argument("--public-key", type=Path, help="independently pinned public key (default: signing-dir/release.pub)")
    parser.add_argument("--config-dir", type=Path, help="existing Sparkplane client configuration directory")
    parser.add_argument("--evidence-dir", type=Path, help="parent directory for a fresh private run directory")
    parser.add_argument("--no-install-client", action="store_true", help="leave the workstation client symlink unchanged")
    parser.add_argument("--codex-model", help="regenerate managed Codex config after a successful apply")
    parser.add_argument("--json", action="store_true", help="emit one result document; progress goes to stderr")
    args = parser.parse_args()
    for name in ["prepared", "signing_dir", "public_key", "config_dir", "evidence_dir"]:
        value = getattr(args, name)
        if value is not None:
            setattr(args, name, value.resolve())
    os.umask(0o077)
    result = {"schema":"sparkplane.workstation-update/v1", "state":"preparing", "host":args.host,
              "appliance_applied":False, "release":None, "evidence":None, "client":None}
    code = 0
    try:
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", args.host) or args.host == "bootstrap":
            raise UpdateError("host must be an existing OpenSSH alias")
        if args.codex_model and not args.apply:
            raise UpdateError("--codex-model requires --apply")
        if platform.system() != "Linux" or platform.machine() not in {"x86_64", "aarch64"}:
            raise UpdateError("workstation updates support Linux x86-64 and ARM64")
        # Ignore ambient CLI action flags; the helper chooses every typed operation.
        for key in list(os.environ):
            if key.startswith("SPARKPLANE_") and key != "SPARKPLANE_CONFIG_DIR":
                del os.environ[key]
        updates = ROOT / "target/spark-updates"
        updates.mkdir(parents=True, exist_ok=True)
        parent = (args.evidence_dir or updates).resolve()
        parent.mkdir(parents=True, exist_ok=True)
        evidence = Path(tempfile.mkdtemp(prefix="run-", dir=parent))
        result["evidence"] = str(evidence)
        progress(f"evidence: {evidence}")
        with (updates / "update.lock").open("a") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise UpdateError("another workstation update is running in this checkout") from error
            update(args, evidence, result)
    except (UpdateError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        result["failed_during"] = result["state"]
        result["state"] = "failed"
        result["error"] = str(error)
        progress(str(error))
        code = 1
    except KeyboardInterrupt:
        result["failed_during"] = result["state"]
        result["state"] = "interrupted"
        progress("interrupted; inspect saved evidence and Spark status before retrying an apply")
        code = 130
    if result["evidence"]:
        write_json(Path(result["evidence"]) / "result.json", result)
    if args.json:
        print(json.dumps(result))
    else:
        print(f"Spark update: {result['state']}")
        if result["release"]:
            print(f"Signed release: {result['release']}")
        if result["evidence"]:
            print(f"Evidence: {result['evidence']}")
        if result["state"] == "previewed":
            print("Apply this signed release with --prepared <release> --apply.")
    return code


if __name__ == "__main__":
    sys.exit(main())
