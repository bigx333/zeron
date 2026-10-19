"""Exercise the reinstall script with fake apps and macOS commands."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "install-nova-macos.sh"


class MacOSInstallTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="nova-install-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.script = self.root / "scripts/install-nova-macos.sh"
        self.script.parent.mkdir()
        source = SCRIPT.read_text()
        replacements = {
            'DEST="/Applications/Zeron Nova.app"': 'DEST="$ROOT/installed/Zeron Nova.app"',
            'BACKUP="/tmp/Zeron Nova.old.app"': 'BACKUP="$ROOT/previous.app"',
            'LOG="/tmp/zeron-nova-install.log"': 'LOG="$ROOT/install.log"',
            'LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"': 'LSREGISTER="$ROOT/bin/lsregister"',
            'plist="/tmp/$label.plist"': 'plist="$ROOT/$label.plist"',
        }
        for old, new in replacements.items():
            self.assertIn(old, source)
            source = source.replace(old, new)
        self.script.write_text(source)
        self.new = self.root / "target/package/Zeron Nova.app"
        self.installed = self.root / "installed/Zeron Nova.app"
        self.backup = self.root / "previous.app"
        for app, version in ((self.new, "new"), (self.installed, "old"), (self.backup, "older")):
            binary = app / "Contents/MacOS/zeron"
            binary.parent.mkdir(parents=True)
            binary.write_text(version)

        # All commands with side effects operate only inside this fixture.
        shim = self.bin / "shim"
        shim.write_text(f"#!{sys.executable}\n" + textwrap.dedent('''\
            import json, os, pathlib, plistlib, shutil, subprocess, sys, time
            root = pathlib.Path(os.environ["INSTALL_TEST_ROOT"])
            name = pathlib.Path(sys.argv[0]).name
            args = sys.argv[1:]
            running = root / "running"
            with (root / "commands.jsonl").open("a") as stream:
                stream.write(json.dumps([name, *args]) + "\\n")
            if name == "launchctl":
                if args[0] != "bootstrap":
                    sys.exit("unexpected launchctl command: " + str(args))
                with open(args[2], "rb") as stream:
                    job = plistlib.load(stream)
                env = os.environ.copy()
                env.update(job["EnvironmentVariables"])
                env["PATH"] = str(root / "bin") + ":" + env["PATH"]
                sys.exit(subprocess.call(job["ProgramArguments"], env=env))
            elif name == "pgrep":
                if running.exists():
                    print("4242")
                else:
                    sys.exit(1)
            elif name == "ps":
                print("1" if "ppid=" in args else "/sbin/launchd")
            elif name == "osascript":
                if (root / "blocked-quit").exists():
                    time.sleep(30)
                else:
                    running.unlink(missing_ok=True)
            elif name == "pkill":
                running.unlink(missing_ok=True)
            elif name == "ditto":
                shutil.copytree(args[0], args[1])
            elif name == "defaults":
                print("0.2.114")
            elif name == "open":
                assert "ZERON_NOVA_INSTALL_DETACHED" not in os.environ
                assert "ZERON_NOVA_INSTALL_DRY" not in os.environ
                running.touch()
            elif name not in ("codesign", "lsregister", "sleep"):
                sys.exit("unexpected shim: " + name)
        '''))
        shim.chmod(0o755)
        for name in ("launchctl", "pgrep", "ps", "osascript", "pkill", "ditto", "defaults", "open", "codesign", "lsregister", "sleep"):
            (self.bin / name).symlink_to(shim.name)

    def run_install(self, *args):
        env = os.environ.copy()
        env.update(
            PATH=str(self.bin) + ":" + env["PATH"],
            INSTALL_TEST_ROOT=str(self.root),
            ZERON_NOVA_INSTALL_DETACHED="1",
            ZERON_NOVA_INSTALL_DRY="1",
        )
        result = subprocess.run(
            ["/bin/bash", str(self.script), *args], env=env,
            text=True, capture_output=True, timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def test_inherited_flags_cannot_skip_launchd_or_turn_install_into_dry_run(self):
        output = self.run_install()
        self.assertIn("done: Zeron Nova is running", output)
        self.assertEqual((self.installed / "Contents/MacOS/zeron").read_text(), "new")
        self.assertEqual((self.backup / "Contents/MacOS/zeron").read_text(), "old")
        older = list(self.root.glob("previous.app.*"))
        self.assertEqual(len(older), 1)
        self.assertEqual((older[0] / "Contents/MacOS/zeron").read_text(), "older")
        self.assertIn('"launchctl", "bootstrap"', (self.root / "commands.jsonl").read_text())

    def test_explicit_dry_run_preserves_apps_and_prior_log(self):
        log = self.root / "install.log"
        log.write_text("previous install log")
        output = self.run_install("--dry-run")
        self.assertIn("done: dry run, nothing changed", output)
        self.assertEqual((self.installed / "Contents/MacOS/zeron").read_text(), "old")
        self.assertEqual((self.backup / "Contents/MacOS/zeron").read_text(), "older")
        self.assertEqual(next(self.root.glob("install.log.*")).read_text(), "previous install log")
        self.assertFalse((self.root / "running").exists())

    def test_blocked_apple_event_cannot_block_install(self):
        (self.root / "running").touch()
        (self.root / "blocked-quit").touch()
        output = self.run_install()
        self.assertIn("Zeron Nova did not quit in 10s; terminating", output)
        self.assertIn("done: Zeron Nova is running", output)
        self.assertIn('"pkill", "-a", "-TERM"', (self.root / "commands.jsonl").read_text())


if __name__ == "__main__":
    unittest.main()
