#!/usr/bin/env bash
# Merge upstream/main into nova through a sync/upstream-<shortsha> PR.
# See NOVA.md. Safe to rerun at any point: each step checks the state it finds.
#
#   scripts/nova/sync-upstream.sh [--dry-run]
set -euo pipefail

REPO=bigx333/zeron
BASE=origin/nova
UPSTREAM=upstream/main

dry_run=0
case "${1:-}" in
  "") ;;
  --dry-run) dry_run=1 ;;
  -h | --help) sed -n '2,5p' "$0"; exit 0 ;;
  *) echo "usage: $0 [--dry-run]" >&2; exit 2 ;;
esac

cd "$(git rev-parse --show-toplevel)"
git fetch --quiet --multiple origin upstream

ahead=$(git rev-list --count "$BASE..$UPSTREAM")
if [[ "$ahead" == 0 ]]; then
  echo "nova is up to date with $UPSTREAM ($(git rev-parse --short "$UPSTREAM")). Nothing to sync."
  exit 0
fi

sha=$(git rev-parse --short "$UPSTREAM")
branch="sync/upstream-$sha"
commits=$(git log --oneline "$BASE..$UPSTREAM")

# Ledger rows sit between the markers in nova's NOVA.md. Column 5 holds the
# upstream hook files as backticked paths.
ledger_hooks() {
  git show "$BASE:NOVA.md" 2>/dev/null |
    sed -n '/<!-- ledger:start -->/,/<!-- ledger:end -->/p' |
    awk -F'|' 'NF >= 7 && $2 !~ /^ *(Feature|-+) *$/ {
      feature = $2; gsub(/^ +| +$/, "", feature)
      hooks = $5
      while (match(hooks, /`[^`]+`/)) {
        print feature "\t" substr(hooks, RSTART + 1, RLENGTH - 2)
        hooks = substr(hooks, RSTART + RLENGTH)
      }
    }'
}

# Per-commit names keep hooks a later commit reverted; the three-dot diff adds
# edits made only inside upstream merge commits, which git log does not list.
touched=$(
  join -t $'\t' -1 2 -2 1 -o 1.1,1.2 \
    <(ledger_hooks | sort -t $'\t' -k2,2) \
    <({
      git log --format= --name-only --no-renames "$BASE..$UPSTREAM"
      git diff --name-only --no-renames "$BASE...$UPSTREAM"
    } | sed '/^$/d' | sort -u) |
    sort -u | awk -F'\t' '{ printf "- %s: `%s`\n", $1, $2 }'
)
touched=${touched:-"- none"}

report() {
  echo "$UPSTREAM ($sha) is $ahead commit(s) ahead of $BASE."
  echo
  echo "Upstream commits:"
  echo "$commits"
  echo
  echo "Ledger hook files these commits touch:"
  echo "$touched"
}

if [[ "$dry_run" == 1 ]]; then
  report
  exit 0
fi

existing=$(gh pr list --repo "$REPO" --head "$branch" --state all \
  --json url,state --jq '.[0] | select(.) | "\(.url) (\(.state | ascii_downcase))"')
if [[ -n "$existing" ]]; then
  echo "PR for $branch already exists: $existing"
  exit 0
fi

if git rev-parse -q --verify MERGE_HEAD >/dev/null; then
  echo "A merge is in progress. Resolve it, commit, then rerun." >&2
  git diff --name-only --diff-filter=U >&2
  exit 1
fi
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  echo "Working tree has uncommitted changes. Commit or set them aside first." >&2
  exit 1
fi

if git rev-parse -q --verify "refs/heads/$branch" >/dev/null; then
  git switch --quiet "$branch"
else
  git switch --quiet --no-track -c "$branch" "$BASE"
fi

if ! git merge-base --is-ancestor "$UPSTREAM" HEAD; then
  if ! git -c rerere.enabled=true -c rerere.autoupdate=true \
    merge --no-ff --no-edit -m "chore(nova): merge upstream main $sha" "$UPSTREAM"; then
    echo >&2
    echo "Merge stopped on conflicts in $branch. Conflicted files:" >&2
    git diff --name-only --diff-filter=U >&2
    echo "Resolve per NOVA.md, commit, then rerun to push and open the PR." >&2
    exit 1
  fi
fi

git push --quiet --no-follow-tags -u origin "$branch"

body=$(
  cat <<EOF
## Intent
Keep nova on upstream main by merge, per NOVA.md.

## What
Merges \`$UPSTREAM\` at \`$sha\` into \`nova\` ($ahead commit(s)).

### Upstream commits
\`\`\`
$commits
\`\`\`

### Ledger hook files these commits touch
$touched

## Checks
- [ ] CI green
- [ ] Nova hooks in the files above still apply
EOF
)

gh pr create --repo "$REPO" --base nova --head "$branch" \
  --title "chore(nova): sync upstream main $sha" --body "$body"
