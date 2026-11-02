"""Exercise the reinstall script with fake apps and macOS commands."""

import os
import plistlib
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "install-nova-macos.sh"
INFO_PLIST = Path(__file__).resolve().parents[2] / "dist/macos/Info.plist"


class MacOSInstallTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="nova-install-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "R&D <nova>"
        self.bin = self.root / "bin"
        self.bin.mkdir(parents=True)
        self.script = self.root / "scripts/install-nova-macos.sh"
        self.script.parent.mkdir()
        source = SCRIPT.read_text()
        replacements = {
            'DEST="/Applications/$APP_NAME.app"': 'DEST="$ROOT/installed/$APP_NAME.app"',
            'BACKUP="/tmp/$APP_NAME.old.app"': 'BACKUP="$ROOT/previous.app"',
            'LOG="/tmp/zeron-nova-install.log"': 'LOG="$ROOT/install.log"',
            'LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"': 'LSREGISTER="$ROOT/bin/lsregister"',
            'plist="/tmp/$label.plist"': 'plist="$ROOT/$label.plist"',
        }
        for old, new in replacements.items():
            self.assertIn(old, source)
            source = source.replace(old, new)
        self.script.write_text(source)
        (self.root / "dist/macos").mkdir(parents=True)
        shutil.copy(INFO_PLIST, self.root / "dist/macos/Info.plist")
        self.new = self.root / "target/package/Zeron Nova.app"
        self.installed = self.root / "installed/Zeron Nova.app"
        self.backup = self.root / "previous.app"
        for app, version in ((self.new, "new"), (self.installed, "old"), (self.backup, "older")):
            binary = app / "Contents/MacOS/zeron"
            binary.parent.mkdir(parents=True)
            binary.write_text(version)
            (app / "Contents/MacOS/Zeron Engine").write_text(version)

        # All commands with side effects operate only inside this fixture.
        shim = self.bin / "shim"
        shim.write_text(f"#!{sys.executable}\n" + textwrap.dedent('''\
            import json, os, pathlib, plistlib, shutil, subprocess, sys, time
            root = pathlib.Path(os.environ["INSTALL_TEST_ROOT"])
            name = pathlib.Path(sys.argv[0]).name
            args = sys.argv[1:]
            running = root / "running"
            # The background engine is matched by its own pattern.
            if name == "pkill" and "Zeron Engine" in args[-1]:
                running = root / "engine-running"
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
            elif name == "ps":
                print("1" if "ppid=" in args else "/sbin/launchd")
            elif name == "osascript":
                if (root / "blocked-quit").exists():
                    time.sleep(30)
                else:
                    running.unlink(missing_ok=True)
            elif name == "pkill":
                if not (root / "engine-ignores-term").exists() or "-KILL" in args:
                    running.unlink(missing_ok=True)
            elif name == "ditto":
                shutil.copytree(args[0], args[1])
                if (root / "ditto-fails").exists():
                    sys.exit("ditto: No space left on device")
            elif name == "plutil":
                if args[0] != "-extract" or args[2] != "raw":
                    sys.exit("unexpected plutil command: " + str(args))
                with open(args[3], "rb") as stream:
                    print(plistlib.load(stream)[args[1]])
            elif name == "defaults":
                print("0.2.114")
            elif name == "open":
                assert "ZERON_NOVA_INSTALL_DETACHED" not in os.environ
                assert "ZERON_NOVA_INSTALL_DRY" not in os.environ
                running.touch()
            elif name not in ("codesign", "lsregister"):
                sys.exit("unexpected shim: " + name)
        '''))
        shim.chmod(0o755)
        for name in ("launchctl", "ps", "osascript", "pkill", "ditto", "plutil", "defaults", "open", "codesign", "lsregister"):
            (self.bin / name).symlink_to(shim.name)
        # Shell, not Python: the stop loops call these hundreds of times.
        (self.bin / "sleep").write_text("#!/bin/sh\n")
        (self.bin / "pgrep").write_text(textwrap.dedent('''\
            #!/bin/sh
            case "$*" in *"Zeron Engine"*) f=engine-running ;; *) f=running ;; esac
            [ -e "$INSTALL_TEST_ROOT/$f" ] || exit 1
            echo 4242
        '''))
        for name in ("sleep", "pgrep"):
            (self.bin / name).chmod(0o755)

    def run_install(self, *args, succeeds=True):
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
        self.assertEqual(result.returncode == 0, succeeds, result.stdout + result.stderr)
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

    def commands(self):
        return (self.root / "commands.jsonl").read_text().splitlines()

    def test_background_engine_stops_before_the_swap(self):
        (self.root / "running").touch()
        (self.root / "engine-running").touch()
        output = self.run_install()
        self.assertIn("stopping the background engine", output)
        self.assertIn("done: Zeron Nova is running", output)
        self.assertFalse((self.root / "engine-running").exists())
        self.assertEqual((self.installed / "Contents/MacOS/Zeron Engine").read_text(), "new")
        commands = self.commands()
        stop = next(i for i, c in enumerate(commands) if '"pkill", "-a", "-TERM"' in c and "Zeron Engine" in c)
        quit_app = next(i for i, c in enumerate(commands) if c.startswith('["osascript"'))
        swap = next(i for i, c in enumerate(commands) if c.startswith('["ditto"'))
        self.assertLess(quit_app, stop)
        self.assertLess(stop, swap)

    def test_engine_that_ignores_term_is_killed(self):
        (self.root / "engine-running").touch()
        (self.root / "engine-ignores-term").touch()
        output = self.run_install()
        self.assertIn("the background engine did not stop in 30s; killing it", output)
        self.assertIn("done: Zeron Nova is running", output)
        self.assertTrue(any('"-KILL"' in c and "Zeron Engine" in c for c in self.commands()))

    def test_failed_copy_restores_the_installed_app(self):
        (self.root / "ditto-fails").touch()
        self.run_install(succeeds=False)
        log = (self.root / "install.log").read_text()
        self.assertIn("failed: could not install", log)
        self.assertEqual((self.installed / "Contents/MacOS/zeron").read_text(), "old")
        self.assertEqual((self.installed / "Contents/MacOS/Zeron Engine").read_text(), "old")

    def test_launchd_job_runs_the_checkout_script_from_any_path(self):
        self.run_install()
        job = next(self.root.glob("sh.zeron.nova-install.*.plist"))
        with job.open("rb") as stream:
            arguments = plistlib.load(stream)["ProgramArguments"]
        self.assertEqual(arguments[1], str(self.script))


if __name__ == "__main__":
    unittest.main()
