#!/usr/bin/env bash
# Reinstall Zeron Nova on this Mac: quit every running instance, replace
# /Applications/Zeron Nova.app with the packaged build, and reopen it.
#
# Usage: scripts/install-nova-macos.sh [--build]
#   --build  run scripts/package-macos.sh first (otherwise installs the
#            existing target/package/Zeron Nova.app)
# The previous install is kept at /tmp/Zeron Nova.old.app.
#
# Safe to run from a chat inside Zeron Nova: the swap detaches from the app
# it is about to quit and logs to /tmp/zeron-nova-install.log.

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
  if [[ "${1:-}" == "--build" ]]; then
    "$ROOT/scripts/package-macos.sh"
  fi
  [[ -d "$SRC" ]] || { echo "no build at $SRC; run with --build" >&2; exit 1; }
  codesign --verify "$SRC"

  for pid in $(gui_pids); do
    if is_ancestor "$pid"; then
      # Quitting Nova takes this shell down with it; finish in a new session.
      ZERON_NOVA_INSTALL_DETACHED=1 nohup perl -MPOSIX -e 'POSIX::setsid(); exec @ARGV' \
        "${BASH_SOURCE[0]}" >"$LOG" 2>&1 </dev/null &
      echo "running inside Zeron Nova; it will quit, update and reopen (log: $LOG)"
      exit 0
    fi
  done
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
    echo "Zeron Nova did not quit in 10s; terminating" >&2
    pkill -a -TERM -f "$GUI_PATTERN" || true
    sleep 2
    pkill -a -KILL -f "$GUI_PATTERN" || true
  fi
fi

rm -rf "$BACKUP"
[[ -d "$DEST" ]] && mv "$DEST" "$BACKUP"
ditto "$SRC" "$DEST"
codesign --verify "$DEST"
echo "installed $(defaults read "$DEST/Contents/Info" CFBundleShortVersionString) to $DEST (previous: $BACKUP)"

# -n: a `zeron mcp` child of this bundle makes LaunchServices treat the app
# as already running, so a plain `open` would do nothing.
open -n "$DEST"
for _ in $(seq 1 40); do
  if [[ -n "$(gui_pids)" ]]; then
    echo "Zeron Nova is running (pid $(gui_pids | tr '\n' ' '))"
    exit 0
  fi
  sleep 0.25
done
echo "Zeron Nova did not start; check Console for crashes (previous build: $BACKUP)" >&2
exit 1
