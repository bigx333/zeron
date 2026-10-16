#!/usr/bin/env bash
# Reinstall Zeron Nova on this Mac: quit every running instance, replace
# /Applications/Zeron Nova.app with the packaged build, and reopen it.
#
# Usage: scripts/install-nova-macos.sh [--build] [--dry-run]
#   --build    run scripts/package-macos.sh first (otherwise installs the
#              existing target/package/Zeron Nova.app)
#   --dry-run  go through detection and the detached hand-off, log what
#              would happen, and change nothing
# The previous install is kept at /tmp/Zeron Nova.old.app.
#
# The quit-swap-reopen always runs as a one-shot launchd agent in the
# user's GUI session, writing /tmp/zeron-nova-install.log: nothing ties it
# to Nova (not its process tree, session or coalition), so it survives
# Nova quitting even when this script runs from a chat inside Nova. From a
# terminal, the script follows that log until the job ends.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/target/package/Zeron Nova.app"
DEST="/Applications/Zeron Nova.app"
BACKUP="/tmp/Zeron Nova.old.app"
LOG="/tmp/zeron-nova-install.log"
BUNDLE_ID="sh.zeron.app.nova"
# The GUI runs the bare binary; `zeron mcp` and other subcommands launched
# from the bundle are left alone so attached agents keep their tools.
GUI_PATTERN="^$DEST/Contents/MacOS/zeron\$"

# -a: macOS pgrep otherwise hides our own ancestors, i.e. the very Nova
# instance a chat runs this script from.
gui_pids() { pgrep -a -f "$GUI_PATTERN" || true; }

is_ancestor() {
  local pid=$$
  while [[ "$pid" -gt 1 ]]; do
    [[ "$pid" == "$1" ]] && return 0
    pid="$(ps -o ppid= -p "$pid" | tr -d ' ')"
  done
  return 1
}

if [[ "${ZERON_NOVA_INSTALL_DETACHED:-}" != 1 ]]; then
  build=0 dry=0
  for arg in "$@"; do
    case "$arg" in
      --build) build=1 ;;
      --dry-run) dry=1 ;;
      *) echo "unknown argument: $arg" >&2; exit 2 ;;
    esac
  done
  [[ $build == 1 ]] && "$ROOT/scripts/package-macos.sh"
  [[ -d "$SRC" ]] || { echo "no build at $SRC; run with --build" >&2; exit 1; }
  codesign --verify "$SRC"

  inside=0
  for pid in $(gui_pids); do is_ancestor "$pid" && inside=1; done

  # Hand the swap to launchd as a one-shot agent. A process merely
  # reparented to launchd still died with Nova once (killed before logging
  # anything), so the job must not start life under Nova at all.
  label="sh.zeron.nova-install"
  domain="gui/$(id -u)"
  plist="/tmp/$label.plist"
  launchctl bootout "$domain/$label" 2>/dev/null || true
  cat >"$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$label</string>
  <key>ProgramArguments</key>
  <array><string>/bin/bash</string><string>$ROOT/scripts/install-nova-macos.sh</string></array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>ZERON_NOVA_INSTALL_DETACHED</key><string>1</string>
    <key>ZERON_NOVA_INSTALL_DRY</key><string>$dry</string>
    <key>PATH</key><string>/usr/bin:/bin:/usr/sbin:/sbin</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
  <key>AbandonProcessGroup</key><true/>
  <key>StandardOutPath</key><string>/dev/null</string>
  <key>StandardErrorPath</key><string>$LOG.launchd</string>
</dict>
</plist>
PLIST
  rm -f "$LOG"
  launchctl bootstrap "$domain" "$plist"
  for _ in $(seq 1 40); do [[ -s "$LOG" ]] && break; sleep 0.25; done
  [[ -s "$LOG" ]] || { echo "the install job did not start (see $LOG.launchd)" >&2; exit 1; }
  job="$(head -1 "$LOG" | sed -n 's/.* job \([0-9]*\) started.*/\1/p')"

  if [[ $inside == 1 ]]; then
    echo "running inside Zeron Nova; job $job will quit, update and reopen it (log: $LOG)"
    exit 0
  fi
  # From a terminal: follow the job's log until it finishes.
  while kill -0 "$job" 2>/dev/null; do sleep 0.25; done
  cat "$LOG"
  tail -n 1 "$LOG" | grep -q '^done:' && exit 0 || exit 1
fi

# ── The detached job ────────────────────────────────────────────────────
exec >"$LOG" 2>&1
echo "$(date '+%F %T') job $$ started: $SRC -> $DEST"
trap 'echo "failed: line $LINENO (previous build: $BACKUP)"' ERR
for signal in TERM HUP INT; do
  # shellcheck disable=SC2064
  trap "echo \"failed: killed by SIG$signal (previous build: $BACKUP)\"; exit 1" "$signal"
done

if [[ "${ZERON_NOVA_INSTALL_DRY:-0}" == 1 ]]; then
  echo "dry run: Nova running as pid(s): $(gui_pids | tr '\n' ' ')"
  # Outlive the caller and anything that cleans up after it.
  sleep 3
  echo "dry run: job $$ still alive, parent $(ps -o ppid= -p $$ | tr -d ' ') ($(ps -o comm= -p "$(ps -o ppid= -p $$ | tr -d ' ')"))"
  echo "done: dry run, nothing changed"
  exit 0
fi

if [[ -n "$(gui_pids)" ]]; then
  echo "quitting Zeron Nova…"
  # Each AppleScript quit reaches one instance; repeat until all are gone.
  for _ in $(seq 1 40); do
    [[ -z "$(gui_pids)" ]] && break
    osascript -e "tell application id \"$BUNDLE_ID\" to quit" >/dev/null 2>&1 || true
    sleep 0.25
  done
  if [[ -n "$(gui_pids)" ]]; then
    echo "Zeron Nova did not quit in 10s; terminating"
    pkill -a -TERM -f "$GUI_PATTERN" || true
    sleep 2
    pkill -a -KILL -f "$GUI_PATTERN" || true
    sleep 0.5
  fi
  [[ -z "$(gui_pids)" ]] || { echo "failed: Zeron Nova is still running"; exit 1; }
fi

rm -rf "$BACKUP"
if [[ -d "$DEST" ]]; then mv "$DEST" "$BACKUP"; fi
ditto "$SRC" "$DEST"
codesign --verify "$DEST"
echo "installed $(defaults read "$DEST/Contents/Info" CFBundleShortVersionString) ($(stat -f '%Sm' "$DEST/Contents/MacOS/zeron")) to $DEST (previous: $BACKUP)"

# -n: a `zeron mcp` child of this bundle makes LaunchServices treat the app
# as already running, so a plain `open` would do nothing.
open -n "$DEST"
for _ in $(seq 1 40); do
  if [[ -n "$(gui_pids)" ]]; then
    echo "done: Zeron Nova is running (pid $(gui_pids | tr '\n' ' '))"
    exit 0
  fi
  sleep 0.25
done
echo "failed: Zeron Nova did not start; check Console for crashes (previous build: $BACKUP)"
exit 1
