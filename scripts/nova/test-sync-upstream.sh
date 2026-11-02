#!/usr/bin/env bash
# Runs sync-upstream.sh against throwaway repos and a stub gh.
#
#   scripts/nova/test-sync-upstream.sh
set -euo pipefail

SCRIPT="$(cd "$(dirname "$0")" && pwd)/sync-upstream.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1

fail() { echo "FAIL: $*" >&2; exit 1; }
expect_in() { [[ "$2" == *"$1"* ]] || fail "expected '$1' in output:"$'\n'"$2"; }

mkdir -p "$tmp/bin"
cat >"$tmp/bin/gh" <<'EOF'
#!/usr/bin/env bash
state="$GH_STUB_DIR/pr"
case "$1 $2" in
  "pr list") [[ -f "$state" ]] && echo "$(cat "$state") (open)"; exit 0 ;;
  "pr create") printf '%s\n' "$@" >"$GH_STUB_DIR/create-args"
    echo "https://github.com/bigx333/zeron/pull/1" | tee "$state" ;;
  *) echo "unexpected gh $*" >&2; exit 1 ;;
esac
EOF
chmod +x "$tmp/bin/gh"
export PATH="$tmp/bin:$PATH" GH_STUB_DIR="$tmp"

git init -q -b main "$tmp/up"
cd "$tmp/up"
printf 'a\n' >hook.rs
printf 'x\n' >other.rs
git add . && git commit -qm "upstream base"
git clone -q --bare "$tmp/up" "$tmp/origin.git"
git -C "$tmp/origin.git" branch nova main

git clone -q "$tmp/origin.git" "$tmp/work"
cd "$tmp/work"
git remote add upstream "$tmp/up"
git switch -q nova
cat >NOVA.md <<'EOF'
<!-- ledger:start -->
| Feature | PR | Nova-only files | Upstream hook files | Notes |
| --- | --- | --- | --- | --- |
| Pool | #9 | `nova/pool.rs` | `hook.rs`, `missing.rs` | |
| Mover | #10 | | `other.rs` | |
| Merger | #11 | | `merged.rs` | |
<!-- ledger:end -->
EOF
git add NOVA.md && git commit -qm "nova ledger" && git push -q origin nova

out=$("$SCRIPT" --dry-run)
expect_in "Nothing to sync" "$out"

git -C "$tmp/up" mv other.rs moved.rs
git -C "$tmp/up" commit -qm "upstream renames other"
git -C "$tmp/up" tag -a v999.0.0 -m release
git -C "$tmp/up" switch -qc side
git -C "$tmp/up" commit -q --allow-empty -m "upstream side work"
git -C "$tmp/up" switch -q main
git -C "$tmp/up" merge -q --no-ff --no-commit side >/dev/null
printf 'm\n' >"$tmp/up/merged.rs"
git -C "$tmp/up" add merged.rs
git -C "$tmp/up" commit -qm "upstream merge edits merged.rs"
printf 'b\n' >"$tmp/up/hook.rs"
git -C "$tmp/up" commit -qam "upstream edits hook"
sha=$(git -C "$tmp/up" rev-parse --short HEAD)

head_before=$(git rev-parse HEAD)
out=$("$SCRIPT" --dry-run)
expect_in "4 commit(s) ahead" "$out"
expect_in "upstream edits hook" "$out"
expect_in "- Pool: \`hook.rs\`" "$out"
expect_in "- Mover: \`other.rs\`" "$out"
expect_in "- Merger: \`merged.rs\`" "$out"
[[ "$out" != *missing.rs* ]] || fail "dry run flagged an untouched hook"
[[ "$(git rev-parse HEAD)" == "$head_before" ]] || fail "dry run moved HEAD"
git ls-remote --exit-code origin "sync/upstream-$sha" >/dev/null && fail "dry run pushed"

out=$("$SCRIPT")
expect_in "pull/1" "$out"
git ls-remote --exit-code origin "sync/upstream-$sha" >/dev/null || fail "branch not pushed"
git -C "$tmp/origin.git" merge-base --is-ancestor "$sha" "sync/upstream-$sha" || fail "pushed branch lacks upstream"
git ls-remote --exit-code --tags origin >/dev/null && fail "tags pushed"
grep -A1 -x -- "--base" "$tmp/create-args" | grep -qx nova || fail "PR not based on nova"

out=$("$SCRIPT")
expect_in "already exists: https://github.com/bigx333/zeron/pull/1" "$out"

rm "$tmp/pr"
git switch -q nova
printf 'nova\n' >hook.rs
git commit -qam "nova edits hook" && git push -q origin nova
git config push.followTags true
printf 'c\n' >"$tmp/up/hook.rs"
git -C "$tmp/up" commit -qam "upstream edits hook again"
csha=$(git -C "$tmp/up" rev-parse --short HEAD)

set +e
out=$("$SCRIPT" 2>&1)
rc=$?
set -e
[[ "$rc" != 0 ]] || fail "conflict exited 0"
expect_in "hook.rs" "$out"
git rev-parse -q --verify MERGE_HEAD >/dev/null || fail "merge not left in progress"
git ls-remote --exit-code origin "sync/upstream-$csha" >/dev/null && fail "conflicted branch pushed"

printf 'resolved\n' >hook.rs
git add hook.rs && git commit -q --no-edit
out=$("$SCRIPT")
expect_in "pull/1" "$out"
git ls-remote --exit-code origin "sync/upstream-$csha" >/dev/null || fail "resolved branch not pushed"
git ls-remote --exit-code --tags origin >/dev/null && fail "tags pushed with push.followTags"

echo "ok: sync-upstream.sh"
