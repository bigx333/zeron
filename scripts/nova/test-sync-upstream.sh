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
run_failing() {
  set +e
  out=$("$SCRIPT" "$@" 2>&1)
  rc=$?
  set -e
  [[ "$rc" != 0 ]] || fail "expected non-zero exit, got 0:"$'\n'"$out"
}
create_arg() { grep -A1 -x -- "$1" "$tmp/create-args" | tail -n1; }
on_nova() { [[ "$(git branch --show-current)" == nova ]] || fail "caller checkout left nova"; }

mkdir -p "$tmp/bin"
cat >"$tmp/bin/gh" <<'EOF'
#!/usr/bin/env bash
state_file="$GH_STUB_DIR/pr"
case "$1 $2" in
  "pr list")
    want=all prev=
    for a in "$@"; do [[ "$prev" == --state ]] && want=$a; prev=$a; done
    [[ -f "$state_file" ]] || exit 0
    read -r url state <"$state_file"
    [[ "$want" == all || "$want" == "$state" ]] && echo "$url"
    exit 0 ;;
  "pr create")
    printf '%s\n' "$@" >"$GH_STUB_DIR/create-args"
    echo "https://github.com/bigx333/zeron/pull/1 open" >"$state_file"
    echo "https://github.com/bigx333/zeron/pull/1" ;;
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
origin="$tmp/github.com/bigx333/zeron.git"
git clone -q --bare "$tmp/up" "$origin"
git -C "$origin" branch nova main

git clone -q "$origin" "$tmp/work"
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
wt_root="$(git rev-parse --path-format=absolute --git-common-dir)/nova-sync"

out=$("$SCRIPT" --dry-run)
expect_in "Nothing to sync" "$out"

git config remote.origin.pushurl "$tmp/up"
run_failing
expect_in "Refusing to sync" "$out"
git config --unset remote.origin.pushurl

git -C "$tmp/up" mv other.rs moved.rs
git -C "$tmp/up" commit -qm "upstream renames other"
git -C "$tmp/up" tag -a v999.0.0 -m release
git -C "$tmp/up" switch -qc side
git -C "$tmp/up" commit -q --allow-empty -m "upstream side work"
git -C "$tmp/up" switch -q main
git -C "$tmp/up" merge -q --no-ff --no-commit side >/dev/null 2>&1
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
[[ -z "$(git tag -l)" ]] || fail "upstream tags fetched: $(git tag -l)"

out=$("$SCRIPT")
expect_in "pull/1" "$out"
on_nova
git ls-remote --exit-code origin "sync/upstream-$sha" >/dev/null || fail "branch not pushed"
git -C "$origin" merge-base --is-ancestor "$sha" "sync/upstream-$sha" || fail "pushed branch lacks upstream"
git ls-remote --exit-code --tags origin >/dev/null && fail "tags pushed"
[[ "$(create_arg --repo)" == bigx333/zeron ]] || fail "PR not on bigx333/zeron"
[[ "$(create_arg --base)" == nova ]] || fail "PR not based on nova"
[[ ! -d "$wt_root/upstream-$sha" ]] || fail "sync worktree left behind after success"

out=$("$SCRIPT")
expect_in "already open: https://github.com/bigx333/zeron/pull/1" "$out"

echo "https://github.com/bigx333/zeron/pull/1 closed" >"$tmp/pr"
rm "$tmp/create-args"
out=$("$SCRIPT")
[[ -f "$tmp/create-args" ]] || fail "closed PR blocked a new one"

rm "$tmp/pr"
printf 'nova\n' >hook.rs
git commit -qam "nova edits hook" && git push -q origin nova
git config push.followTags true
printf 'c\n' >"$tmp/up/hook.rs"
git -C "$tmp/up" commit -qam "upstream edits hook again"
git tag -a v998.0.0 -m local origin/nova
csha=$(git -C "$tmp/up" rev-parse --short HEAD)
wt="$wt_root/upstream-$csha"

run_failing
expect_in "hook.rs" "$out"
on_nova
git -C "$wt" rev-parse -q --verify MERGE_HEAD >/dev/null || fail "merge not left in progress"
git ls-remote --exit-code origin "sync/upstream-$csha" >/dev/null && fail "conflicted branch pushed"

run_failing
expect_in "Conflicted files" "$out"

printf 'resolved\n' >"$wt/hook.rs"
git -C "$wt" add hook.rs && git -C "$wt" commit -q --no-edit
out=$("$SCRIPT")
expect_in "pull/1" "$out"
on_nova
git ls-remote --exit-code origin "sync/upstream-$csha" >/dev/null || fail "resolved branch not pushed"
git ls-remote --exit-code --tags origin >/dev/null && fail "tags pushed with push.followTags"

rm "$tmp/pr"
git branch -q -D "sync/upstream-$csha"
git push -q origin --delete "sync/upstream-$csha"
run_failing
expect_in "rerere resolved all conflicts" "$out"
[[ "$(cat "$wt/hook.rs")" == resolved ]] || fail "rerere did not replay the resolution"
git -C "$wt" commit -q --no-edit
out=$("$SCRIPT")
expect_in "pull/1" "$out"
on_nova

echo "ok: sync-upstream.sh"
