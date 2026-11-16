#!/usr/bin/env python3
"""Install a standalone Zuse CLI and its browser app on Linux or macOS."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import tempfile
import uuid


NAMES = ("zuse", "zuse-server")
MARKER = ".installed-by-zuse-cli"


def verify(package):
    manifest = json.loads((package / "manifest.json").read_text())
    if manifest.get("format") != 1 or manifest.get("kind") != "cli":
        raise ValueError("Expected a standalone Zuse CLI package")
    files = {p.relative_to(package).as_posix(): p for p in package.rglob("*") if p.is_file() and not p.is_symlink()}
    links = {p.relative_to(package).as_posix(): os.readlink(p) for p in package.rglob("*") if p.is_symlink()}
    if set(files) != set(manifest["files"]) | {"manifest.json"} or links != manifest.get("links", {}):
        raise ValueError("Package inventory mismatch")
    if links != {"bin/zuse-server": "zuse"}:
        raise ValueError("Unexpected package links")
    for relative, digest in manifest["files"].items():
        with files[relative].open("rb") as stream:
            if hashlib.file_digest(stream, "sha256").hexdigest() != digest:
                raise ValueError("Package checksum mismatch: " + relative)
    if not (package / "bin/zuse").is_file() or not (package / "share/zuse/web/index.html").is_file():
        raise ValueError("Package is missing the command or browser app")
    return manifest


def owned_link(link, root, name):
    # Keep the stable current path; resolving it would pin a launcher to one version.
    return link.is_symlink() and Path(os.readlink(link)) == root / "current/bin" / name


def replace_link(link, target):
    pending = link.with_name(".zuse-" + uuid.uuid4().hex)
    try:
        pending.symlink_to(target)
        os.replace(pending, link)
    finally:
        pending.unlink(missing_ok=True)


def install(source, root, bin_dir, launchers=True):
    manifest = verify(source)
    system = {"Linux": "linux", "Darwin": "macos"}.get(platform.system())
    arch = {"x86_64": "x86_64", "aarch64": "aarch64", "arm64": "aarch64"}.get(platform.machine())
    if not system or (manifest.get("platform"), manifest.get("arch")) != (system, arch):
        raise ValueError("Package does not match this computer")
    root.mkdir(parents=True, exist_ok=True)
    with (root / "install.lock").open("a") as lease:
        fcntl.flock(lease, fcntl.LOCK_EX)
        if launchers:
            for name in NAMES:
                link = bin_dir / name
                if os.path.lexists(link) and not owned_link(link, root, name):
                    raise ValueError("Refusing to replace an unrelated launcher: " + str(link))
        if os.path.lexists(root / "current") and not (root / MARKER).is_file():
            raise ValueError("Refusing to replace an unmanaged installation")
        packages = root / "packages"
        packages.mkdir(exist_ok=True)
        digest = hashlib.sha256((source / "manifest.json").read_bytes()).hexdigest()[:16]
        destination = packages / (source.name + "-" + digest)
        if destination.exists():
            verify(destination)
        else:
            with tempfile.TemporaryDirectory(prefix=".install-", dir=root) as temporary:
                staged = Path(temporary) / "package"
                shutil.copytree(source, staged, symlinks=True)
                verify(staged)
                os.rename(staged, destination)
        (root / MARKER).write_text("1\n")
        replace_link(root / "current", destination)
        if launchers:
            bin_dir.mkdir(parents=True, exist_ok=True)
            for name in NAMES:
                replace_link(bin_dir / name, root / "current/bin" / name)
    print("Installed Zuse " + manifest["version"] + " at " + str(destination))
    print("Run zuse serve to open Zuse in your browser, or zuse serve --lan for another device.")
    print("Previous versions and user profiles are preserved. Restart a running server to use this version.")


def uninstall(root, bin_dir):
    if not (root / MARKER).is_file():
        raise ValueError("No managed CLI installation found")
    for name in NAMES:
        link = bin_dir / name
        if owned_link(link, root, name):
            link.unlink()
    print("Removed CLI launchers. Packages and profiles are preserved.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.home() / ".zuse/cli")
    parser.add_argument("--no-launchers", action="store_true", help="Update the active package without changing PATH launchers")
    parser.add_argument("--uninstall", action="store_true")
    args = parser.parse_args()
    bin_dir = Path(os.environ.get("ZUSE_BIN_HOME", Path.home() / ".local/bin")).expanduser().absolute()
    root = args.root.expanduser().absolute()
    if args.uninstall:
        uninstall(root, bin_dir)
    else:
        install(Path(__file__).resolve().parent, root, bin_dir, not args.no_launchers)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError) as error:
        raise SystemExit(str(error)) from error
