#!/usr/bin/env python3
"""Focused source-copy check; never resolves the unrelated viewer/wgpu graph.

Uses the repository's exact manifests, profile, lints, lock and source for the
two small crates. All Cargo artifacts use the common checkout's target cache.
The complete integrated workspace still requires the lead's separate checks.
"""
import os
import hashlib
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib


def main():
    root = Path(__file__).resolve().parents[1]
    snapshot = Path(tempfile.mkdtemp(prefix="foldurora-mcp-check-"))
    for name in ("aurora-mcp", "aurora-session"):
        shutil.copytree(root / "crates" / name, snapshot / "crates" / name)
    manifest = (root / "Cargo.toml").read_text()
    manifest = manifest.replace('members = ["crates/*"]',
                                'members = ["crates/aurora-mcp", "crates/aurora-session"]')
    manifest = manifest.split("[patch.crates-io]", 1)[0]
    (snapshot / "Cargo.toml").write_text(manifest)
    shutil.copy2(root / "Cargo.lock", snapshot / "Cargo.lock")
    digest = hashlib.sha256()
    for source in sorted(snapshot.rglob("*")):
        if source.is_file():
            digest.update(str(source.relative_to(snapshot)).encode())
            digest.update(b"\0")
            digest.update(source.read_bytes())
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=root))
    print(f"Origin commit: {commit}; dirty={dirty}; focused source SHA-256={digest.hexdigest()}", flush=True)
    # Cargo prunes unused workspace packages from a smaller workspace. Resolve
    # only this source copy offline, then prove it introduced no new package or
    # version before running the actual check with --locked.
    with (snapshot / "resolution.json").open("w") as output:
        resolved = subprocess.call(
            ["cargo", "metadata", "--offline", "--format-version", "1",
             "--filter-platform", subprocess.check_output(
                 ["rustc", "-vV"], text=True).split("host: ", 1)[1].splitlines()[0]],
            cwd=snapshot, stdout=output)
    if resolved:
        return resolved
    def packages(path):
        return {(p["name"], p["version"], p.get("source"))
                for p in tomllib.loads(path.read_text())["package"]}
    if not packages(snapshot / "Cargo.lock") <= packages(root / "Cargo.lock"):
        raise SystemExit("focused resolution introduced an unlocked package")
    common = Path(subprocess.check_output(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
        cwd=root, text=True).strip())
    env = os.environ.copy()
    env.setdefault("CARGO_TARGET_DIR", str(common.parent / "target"))
    args = sys.argv[1:] or ["test"]
    if args[0] not in ("test", "check", "clippy", "metadata"):
        raise SystemExit("expected test, check, clippy or metadata")
    command = ["cargo", args[0], "--manifest-path", str(snapshot / "Cargo.toml")]
    if args[0] != "metadata":
        command += ["--profile", "ci", "-p", "aurora-mcp"]
    command += ["--locked", "--offline"] + args[1:]
    print(f"Focused source snapshot: {snapshot}", flush=True)
    print(" ".join(command), flush=True)
    return subprocess.call(command, cwd=root, env=env)


if __name__ == "__main__":
    raise SystemExit(main())
