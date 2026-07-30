#!/bin/bash
# Sync this public-edition checkout to both remotes:
#   1. private repo's public-edition branch (origin)
#   2. public repo's main branch (public-origin) — force-pushed, unrelated git history
#
# Usage:
#   tools/sync-public-edition.sh                        # commit (if needed) + push both, no version bump
#   tools/sync-public-edition.sh -m "commit message"     # non-interactive commit message
#   tools/sync-public-edition.sh 1.2.0                   # also bump version, commit, tag "v1.2.0"
#   tools/sync-public-edition.sh 1.2.0 -m "message"      # combo
#   tools/sync-public-edition.sh --yes                   # skip confirmation prompts
#   tools/sync-public-edition.sh --skip-tests            # skip `npm test` before pushing
#
# Run this from anywhere inside the public-edition checkout — it cd's to the repo root.
# Requires this checkout to actually be on the "public-edition" branch (aborts otherwise).

set -euo pipefail

PRIVATE_REMOTE="origin"
PUBLIC_REMOTE="public-origin"
PUBLIC_URL="https://github.com/strejda603/softube-ms-bridge-public.git"
BRANCH="public-edition"

NEW_VERSION=""
COMMIT_MESSAGE=""
ASSUME_YES=false
SKIP_TESTS=false

while [ $# -gt 0 ]; do
  case "$1" in
    -m|--message)
      COMMIT_MESSAGE="$2"
      shift 2
      ;;
    --yes)
      ASSUME_YES=true
      shift
      ;;
    --skip-tests)
      SKIP_TESTS=true
      shift
      ;;
    *)
      NEW_VERSION="$1"
      shift
      ;;
  esac
done

confirm() {
  if [ "$ASSUME_YES" = true ]; then
    return 0
  fi
  read -r -p "$1 [y/N] " reply
  [ "$reply" = "y" ] || [ "$reply" = "Y" ]
}

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

CURRENT_BRANCH="$(git branch --show-current)"
if [ "$CURRENT_BRANCH" != "$BRANCH" ]; then
  echo "Refusing to run: current branch is '$CURRENT_BRANCH', expected '$BRANCH'."
  exit 1
fi

if ! git remote get-url "$PUBLIC_REMOTE" >/dev/null 2>&1; then
  echo "Adding remote $PUBLIC_REMOTE -> $PUBLIC_URL"
  git remote add "$PUBLIC_REMOTE" "$PUBLIC_URL"
fi

echo "Fetching $PRIVATE_REMOTE/$BRANCH and $PUBLIC_REMOTE/main..."
git fetch "$PRIVATE_REMOTE" "$BRANCH"
git fetch "$PUBLIC_REMOTE" main

DIVERGED="$(git log "HEAD..$PUBLIC_REMOTE/main" --oneline 2>/dev/null || true)"
if [ -n "$DIVERGED" ]; then
  echo
  echo "WARNING: the public repo's main has commits not present here:"
  echo "$DIVERGED"
  echo "These will be LOST — force-pushing overwrites public repo history entirely."
  confirm "Continue anyway?" || { echo "Aborted."; exit 1; }
fi

if [ -n "$NEW_VERSION" ]; then
  echo "Bumping version to $NEW_VERSION..."
  npm pkg set version="$NEW_VERSION"
  git add package.json
fi

if [ -n "$(git status --porcelain)" ]; then
  if [ -z "$COMMIT_MESSAGE" ]; then
    if [ -n "$NEW_VERSION" ]; then
      COMMIT_MESSAGE="Update $NEW_VERSION"
    elif [ "$ASSUME_YES" = true ]; then
      echo "Uncommitted changes present and no -m message given with --yes. Aborting."
      exit 1
    else
      read -r -p "Commit message: " COMMIT_MESSAGE
      [ -n "$COMMIT_MESSAGE" ] || { echo "Empty message, aborting."; exit 1; }
    fi
  fi
  git add -A
  git commit -m "$COMMIT_MESSAGE"
else
  echo "No uncommitted changes."
fi

if [ "$SKIP_TESTS" = false ]; then
  echo "Running tests..."
  npm test
fi

echo
confirm "Push $BRANCH to $PRIVATE_REMOTE?" || { echo "Aborted before push."; exit 1; }
git push "$PRIVATE_REMOTE" "$BRANCH"

confirm "Force-push $BRANCH to $PUBLIC_REMOTE/main?" || { echo "Aborted before public push."; exit 1; }
git push "$PUBLIC_REMOTE" "$BRANCH:main" --force

if [ -n "$NEW_VERSION" ]; then
  git tag "v$NEW_VERSION"
  git push "$PRIVATE_REMOTE" "v$NEW_VERSION"
  git push "$PUBLIC_REMOTE" "v$NEW_VERSION"
  echo "Tagged and pushed v$NEW_VERSION — this triggers the public repo's release build."
fi

echo "Done."
