"""Standalone installation preserves profiles and switches binary/assets together."""
import hashlib
import json
import os
from pathlib import Path
import platform
import tarfile
import unittest
from unittest.mock import patch

import test_package as fixtures
from test_distribution import bootstrap_module


installer = fixtures.load("cli_installer", "install-cli.py") if os.name == "posix" else None


@unittest.skipUnless(platform.system() == "Linux", "Linux archive fixture")
class CliInstallTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.PackagingTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root / "managed cli"
        self.bin_dir = self.fixture.root / "user bin"
        self.package = self.fixture.root / "zuse-cli-0.25.0-linux-x86_64"
        self.assemble(self.package)
        machine = patch.object(platform, "machine", return_value="x86_64")
        machine.start()
        self.addCleanup(machine.stop)

    def assemble(self, destination, version="0.25.0"):
        fixtures.package.assemble_cli(destination, self.fixture.binaries[1], self.fixture.assets,
                                      "linux", "x86_64", version, {})

    def test_install_upgrade_and_uninstall_preserve_previous_package_and_profiles(self):
        profile = self.fixture.root / "profile"
        profile.write_text("user conversations")
        installer.install(self.package, self.root, self.bin_dir)
        old = (self.bin_dir / "zuse").resolve()
        self.assertEqual(os.readlink(self.bin_dir / "zuse-server"), str(self.root / "current/bin/zuse-server"))
        self.assertEqual((self.root / "current/share/zuse/web/app.js").read_text(), "// fixture")
        # Rerunning the same verified installation is safe.
        installer.install(self.package, self.root, self.bin_dir)
        next_package = self.fixture.root / "zuse-cli-0.25.1-linux-x86_64"
        (self.fixture.assets / "app.js").write_text("// new browser")
        self.assemble(next_package, "0.25.1")
        installer.install(next_package, self.root, self.bin_dir, launchers=False)
        self.assertNotEqual(old, (self.bin_dir / "zuse").resolve())
        self.assertTrue(old.is_file())
        self.assertEqual((self.root / "current/share/zuse/web/app.js").read_text(), "// new browser")
        installer.uninstall(self.root, self.bin_dir)
        self.assertFalse(os.path.lexists(self.bin_dir / "zuse"))
        self.assertEqual(profile.read_text(), "user conversations")
        self.assertTrue(old.exists())

    def test_bad_payload_or_unrelated_launcher_cannot_replace_current(self):
        installer.install(self.package, self.root, self.bin_dir)
        before = os.readlink(self.root / "current")
        (self.package / "share/zuse/web/app.js").write_text("tampered")
        with self.assertRaisesRegex(ValueError, "checksum"):
            installer.install(self.package, self.root, self.bin_dir)
        self.assertEqual(os.readlink(self.root / "current"), before)
        (self.bin_dir / "zuse").unlink()
        (self.bin_dir / "zuse").write_text("someone else's CLI")
        installer.uninstall(self.root, self.bin_dir)
        self.assertEqual((self.bin_dir / "zuse").read_text(), "someone else's CLI")
        with self.assertRaisesRegex(ValueError, "unrelated"):
            # Use the untampered installed copy.
            installer.install(Path(before), self.root, self.bin_dir)
        self.assertEqual(os.readlink(self.root / "current"), before)

    def test_bootstrap_selects_cli_archive_and_installs_without_desktop_files(self):
        archive = self.fixture.root / (self.package.name + ".tar.gz")
        with tarfile.open(archive, "w:gz") as bundle:
            bundle.add(self.package, arcname=self.package.name)
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest() + "  " + archive.name + "\n"
        requested = []
        def download(url):
            requested.append(url)
            return checksum.encode() if url.endswith("SHA256SUMS.cli") else archive.read_bytes()
        fake_home = self.fixture.root / "home"
        fake_home.mkdir()
        with patch.dict(os.environ, HOME=str(fake_home), ZUSE_BIN_HOME=str(self.bin_dir)):
            bootstrap = bootstrap_module()
            bootstrap.install("0.25.0", download, cli=True)
            installed = (self.bin_dir / "zuse").resolve()
            self.assertTrue(installed.is_file())
            self.assertTrue((installed.parent.parent / "share/zuse/web/index.html").is_file())
            self.assertFalse((fake_home / ".local/share/applications").exists())
            bootstrap.uninstall(cli=True)
        self.assertTrue(any(url.endswith("SHA256SUMS.cli") for url in requested))
        self.assertFalse(os.path.lexists(self.bin_dir / "zuse"))


if __name__ == "__main__":
    unittest.main()
