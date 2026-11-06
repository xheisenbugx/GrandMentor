#!/usr/bin/env bash
# Publish PR evidence (screenshots, GIFs) to the orphan `media` branch instead of committing it
# to the code history, and print the raw URLs to embed in the PR description.
#
#   tools/screenshots/publish-evidence.sh <branch-name> <file>...
#   tools/screenshots/publish-evidence.sh fix/drag-and-drop /tmp/gm-shots/before.png /tmp/gm-shots/after.gif
#
# Files land in `prs/<branch-name with / replaced by ->/` on `media` and are pushed to origin.
# Re-running with the same file name replaces it. Needs push access to origin.
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "usage: $0 <branch-name> <file>..." >&2
  exit 2
fi

branch="$1"; shift
slug="${branch//\//-}"
repo_url="https://raw.githubusercontent.com/xheisenbugx/GrandMentor/media/prs/$slug"

for f in "$@"; do
  [ -f "$f" ] || { echo "not a file: $f" >&2; exit 2; }
done

root="$(git rev-parse --show-toplevel)"
tmp="$(mktemp -d)"
wt="$tmp/media"
cleanup() {
  git -C "$root" worktree remove --force "$wt" >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT

if git -C "$root" fetch -q origin media 2>/dev/null; then
  git -C "$root" worktree add -q --detach "$wt" FETCH_HEAD
else
  echo "origin has no media branch yet; creating it" >&2
  git -C "$root" worktree add -q --orphan -b "media-tmp-$$" "$wt"
fi

mkdir -p "$wt/prs/$slug"
for f in "$@"; do
  cp "$f" "$wt/prs/$slug/"
done

git -C "$wt" add "prs/$slug"
if git -C "$wt" diff --cached --quiet; then
  echo "nothing new to publish" >&2
else
  git -C "$wt" commit -q -m "media: evidence for $branch"
  git -C "$wt" push -q origin HEAD:refs/heads/media
fi
git -C "$root" branch -D "media-tmp-$$" >/dev/null 2>&1 || true

for f in "$@"; do
  echo "$repo_url/$(basename "$f")"
done
