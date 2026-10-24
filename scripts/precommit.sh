#!/usr/bin/env bash
set -uo pipefail

REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT" || exit 1
echo "=== pre-commit: make lint ==="
make lint || { echo "FAIL: make lint"; exit 1; }

echo "=== pre-commit: make test ==="
make test || { echo "FAIL: make test"; exit 1; }

echo ""
echo "pre-commit: all checks passed"
exit 0
