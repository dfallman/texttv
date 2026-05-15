#!/bin/sh
# Install texttv's git hooks into the current repo's .git/hooks/.
# Idempotent — running it again replaces the existing hook with the same
# tracked source. Pass --uninstall to remove.

set -e

REPO_ROOT=$(git rev-parse --show-toplevel)
HOOKS_DIR=$(git rev-parse --git-dir)/hooks
SRC_DIR="$REPO_ROOT/tools/git-hooks"

if [ "$1" = "--uninstall" ]; then
    rm -f "$HOOKS_DIR/post-commit"
    echo "uninstalled post-commit"
    exit 0
fi

install -m 0755 "$SRC_DIR/post-commit" "$HOOKS_DIR/post-commit"
echo "installed: $HOOKS_DIR/post-commit"
