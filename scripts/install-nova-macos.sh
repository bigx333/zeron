#!/usr/bin/env bash
# Reinstall Zeron Nova on this Mac: quit every running instance and its
# background engine, replace /Applications/Zeron Nova.app with the packaged
# build, and reopen it (the app starts the new engine).
#
# Usage: scripts/install-nova-macos.sh [--build] [--dry-run]
#   --build    run scripts/package-macos.sh first (otherwise installs the
#              existing target/package/Zeron Nova.app)
#   --dry-run  go through detection and the detached hand-off, log what
#              would happen, and change nothing
# The previous install is kept at /tmp/Zeron Nova.old.app. Older backups
# and install logs are renamed rather than deleted.
#
# The quit-swap-reopen always runs as a one-shot launchd agent in the
# user's GUI session, writing /tmp/zeron-nova-install.log: nothing ties it
# to Nova (not its process tree, session or coalition), so it survives
# Nova and its engine stopping even when this script runs from a chat
# inside Nova (agents run under the engine). From a terminal, the script
# follows that log until the job ends.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/target/package/Zeron Nova.app"
DEST="/Applications/Zeron Nova.app"
BACKUP="/tmp/Zeron Nova.old.app"
LOG="/tmp/zeron-nova-install.log"
BUNDLE_ID="sh.zeron.app.nova"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
# The GUI runs the bare binary; `zeron mcp` and other subcommands launched
# from the bundle are left alone so attached agents keep their tools.
GUI_PATTERN="^$DEST/Contents/MacOS/zeron\$"
# The background engine outlives the app. Left running, it would keep
# serving the reopened app from the moved backup bundle: it only restarts
# into a newer installed *version*, and a rebuild usually keeps the version.
ENGINE_PATTERN="^$DEST/Contents/MacOS/Zeron Engine( |\$)"

# -a: macOS pgrep otherwise hides our own ancestors, i.e. the very Nova
# instance (or engine) a chat runs this script from.
gui_pids() { pgrep -a -f "$GUI_PATTERN" || true; }
engine_pids() { pgrep -a -f "$ENGINE_PATTERN" || true; }

is_ancestor() {
  local pid=$$
  while [[ "$pid" -gt 1 ]]; do
    [[ "$pid" == "$1" ]] && return 0
    pid="$(ps -o ppid= -p "$pid" | tr -d ' ')"
  done
  return 1
}

# Older installers leaked these flags into the app they reopened. A later
# install from that app then skipped launchd and died when the app quit.
# Only an explicit argument to the launchd job may select the worker path.
unset ZERON_NOVA_INSTALL_DETACHED ZERON_NOVA_INSTALL_DRY
build=0 dry=0 worker=0
for arg in "$@"; do
  case "$arg" in
    --build) build=1 ;;
    --dry-run) dry=1 ;;
    --launchd-job) worker=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

if [[ $worker == 0 ]]; then
  [[ $build == 1 ]] && "$ROOT/scripts/package-macos.sh"
  [[ -d "$SRC" ]] || { echo "no build at $SRC; run with --build" >&2; exit 1; }
  codesign --verify "$SRC"

  inside=0
  for pid in $(gui_pids) $(engine_pids); do is_ancestor "$pid" && inside=1; done

  # Hand the swap to launchd as a one-shot agent. A process merely
  # reparented to launchd still died with Nova once (killed before logging
  # anything), so the job must not start life under Nova at all.
  # Reusing a label requires booting out its old job, whose app coalition
  # may still contain the Nova instance running this installer.
  label="sh.zeron.nova-install.$(id -u).$$"
  domain="gui/$(id -u)"
  plist="/tmp/$label.plist"
  cat >"$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$label</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/bash</string><string>$ROOT/scripts/install-nova-macos.sh</string>
    <string>--launchd-job</string>
    $([[ $dry == 1 ]] && echo '<string>--dry-run</string>')
  </array>
  <key>EnvironmentVariables</key>
  <dict>
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
  if [[ -e "$LOG" ]]; then mv "$LOG" "$LOG.$(date '+%Y%m%d-%H%M%S').$$"; fi
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

parent="$(ps -o ppid= -p $$ | tr -d ' ')"
[[ "$parent" == 1 ]] || { echo "failed: install job must be started by launchd (parent $parent)"; exit 1; }

if [[ $dry == 1 ]]; then
  echo "dry run: Nova running as pid(s): $(gui_pids | tr '\n' ' ')"
  echo "dry run: engine running as pid(s): $(engine_pids | tr '\n' ' ')"
  # Outlive the caller and anything that cleans up after it.
  sleep 3
  echo "dry run: job $$ still alive, parent $(ps -o ppid= -p $$ | tr -d ' ') ($(ps -o comm= -p "$(ps -o ppid= -p $$ | tr -d ' ')"))"
  echo "done: dry run, nothing changed"
  exit 0
fi

if [[ -n "$(gui_pids)" ]]; then
  echo "quitting Zeron Nova…"
  # AppleScript can block while the app is busy. Keep it in a child process
  # so the ten-second deadline applies to the whole quit, not each request.
  quit_pid=""
  for _ in $(seq 1 40); do
    [[ -z "$(gui_pids)" ]] && break
    if [[ -z "$quit_pid" ]] || ! kill -0 "$quit_pid" 2>/dev/null; then
      if [[ -n "$quit_pid" ]]; then wait "$quit_pid" || true; fi
      osascript -e "tell application id \"$BUNDLE_ID\" to quit" >/dev/null 2>&1 &
      quit_pid=$!
    fi
    sleep 0.25
  done
  if [[ -n "$quit_pid" ]]; then
    kill -TERM "$quit_pid" 2>/dev/null || true
    wait "$quit_pid" 2>/dev/null || true
  fi
  if [[ -n "$(gui_pids)" ]]; then
    echo "Zeron Nova did not quit in 10s; terminating"
    pkill -a -TERM -f "$GUI_PATTERN" || true
    sleep 2
    pkill -a -KILL -f "$GUI_PATTERN" || true
    sleep 0.5
  fi
  [[ -z "$(gui_pids)" ]] || { echo "failed: Zeron Nova is still running"; exit 1; }
fi

# After the app: a running app would start the engine again. SIGTERM lets it
# drain (its sessions stop with it); the reopened app starts the new one.
if [[ -n "$(engine_pids)" ]]; then
  echo "stopping the background engine…"
  pkill -a -TERM -f "$ENGINE_PATTERN" || true
  for _ in $(seq 1 120); do
    [[ -z "$(engine_pids)" ]] && break
    sleep 0.25
  done
  if [[ -n "$(engine_pids)" ]]; then
    echo "the background engine did not stop in 30s; killing it"
    pkill -a -KILL -f "$ENGINE_PATTERN" || true
    sleep 0.5
  fi
  [[ -z "$(engine_pids)" ]] || { echo "failed: the background engine is still running"; exit 1; }
fi

if [[ -e "$BACKUP" ]]; then mv "$BACKUP" "$BACKUP.$(date '+%Y%m%d-%H%M%S').$$"; fi
if [[ -d "$DEST" ]]; then mv "$DEST" "$BACKUP"; fi
ditto "$SRC" "$DEST"
codesign --verify "$DEST"
cmp "$SRC/Contents/MacOS/zeron" "$DEST/Contents/MacOS/zeron"
if [[ -e "$SRC/Contents/MacOS/Zeron Engine" ]]; then
  cmp "$SRC/Contents/MacOS/Zeron Engine" "$DEST/Contents/MacOS/Zeron Engine"
fi
echo "installed $(defaults read "$DEST/Contents/Info" CFBundleShortVersionString) ($(stat -f '%Sm' "$DEST/Contents/MacOS/zeron")) to $DEST (previous: $BACKUP)"

# Leave $DEST the only bundle LaunchServices knows for $BUNDLE_ID. Banners
# take their icon from whichever registered copy the id resolves to, and a
# stale build or backup (e.g. one from before the Nova icon) wins often
# enough to put the old logo on notifications. A path can hold several
# records, so each one is unregistered separately.
"$LSREGISTER" -dump 2>/dev/null \
  | awk -v id="$BUNDLE_ID" '/^path:/ { sub(/^path: */, ""); sub(/ \(0x[0-9a-f]+\)$/, ""); path = $0 }
                            /^identifier:/ && $2 == id { print path }' \
  | while IFS= read -r stale; do
      [[ "$stale" == "$DEST" ]] && continue
      "$LSREGISTER" -u "$stale" 2>/dev/null && echo "unregistered stale copy: $stale"
    done || true
"$LSREGISTER" -f "$DEST"

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
