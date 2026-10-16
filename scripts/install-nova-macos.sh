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
# The quit-swap-reopen always runs in a detached job (its own session,
# reparented to launchd) that writes /tmp/zeron-nova-install.log, so it
# survives Nova quitting even when this script runs from a chat inside
# Nova. From a terminal, the script follows that log until the job ends.

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

  # Hand the swap to a job in its own session, reparented to launchd. The
  # perl parent returns only once the child has called setsid and exec'd
  # (the pipe closes on exec), so whatever tears down this script's process
  # group afterwards (a chat's command runner, Nova quitting) can't reach it.
  job="$(ZERON_NOVA_INSTALL_DETACHED=1 ZERON_NOVA_INSTALL_DRY="$dry" perl -MPOSIX -e '
    pipe(my $r, my $w) or die "pipe: $!";
    my $pid = fork // die "fork: $!";
    if ($pid) { close $w; <$r>; print "$pid\n"; exit 0 }
    close $r;
    POSIX::setsid() or die "setsid: $!";
    open STDIN, "<", "/dev/null";
    open STDOUT, ">", "/dev/null";
    open STDERR, ">", "/dev/null";
    exec @ARGV or die "exec: $!";
  ' "${BASH_SOURCE[0]}")"

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

if [[ "${ZERON_NOVA_INSTALL_DRY:-0}" == 1 ]]; then
  echo "dry run: Nova running as pid(s): $(gui_pids | tr '\n' ' ')"
  # Outlive the caller and anything that cleans up after it.
  sleep 3
  echo "dry run: job $$ still alive in session $(ps -o sess= -p $$ | tr -d ' '), parent $(ps -o ppid= -p $$ | tr -d ' ')"
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
