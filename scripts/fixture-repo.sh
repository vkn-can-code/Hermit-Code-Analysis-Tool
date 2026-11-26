#!/usr/bin/env bash
# Builds a git repo from fixtures/<name>/{base,head}: `main` = base, `feature` = head.
# Usage: scripts/fixture-repo.sh fixtures/python_sqli /tmp/demo
set -euo pipefail
src="$(cd "$1" && pwd)"; dest="$2"
rm -rf "$dest"; mkdir -p "$dest"; cd "$dest"
export GIT_AUTHOR_NAME=hermit GIT_AUTHOR_EMAIL=hermit@localhost GIT_COMMITTER_NAME=hermit GIT_COMMITTER_EMAIL=hermit@localhost
git init -q -b main
cp -R "$src/base/." .
git add -A && git commit -qm "base"
git checkout -qb feature
find . -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
cp -R "$src/head/." .
git add -A && git commit -qm "head"
echo "$dest"
