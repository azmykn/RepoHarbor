#!/usr/bin/env bash
# One-shot: finish history wipe + push RepoHarbor (run in a normal terminal — Cursor agent cannot commit).
set -euo pipefail
cd "$(dirname "$0")/.."

test "$(basename "$(pwd)")" = "Orrery"
git rev-parse --verify repoharbor-main >/dev/null

# Commit orphan tree if needed
if ! git rev-parse --verify HEAD >/dev/null 2>&1; then
  git add -A
  git commit -m "$(cat <<'EOF'
Initial commit: RepoHarbor

Independent DigitsCode product (not a fork continuation). Includes software
originally published as Orrery by Seb Burrell (MIT).
EOF
)"
fi

git branch -M main
# Drop local pre-wipe history refs
git branch -D feat/digitscode-roadmap-p0-p3 2>/dev/null || true
# Point origin at independent RepoHarbor (keep old fork as orrery-old)
if git remote get-url origin 2>/dev/null | grep -q 'azmykn/Orrery'; then
  git remote rename origin orrery-old
fi
if ! git remote get-url origin >/dev/null 2>&1; then
  git remote add origin git@github.com:azmykn/RepoHarbor.git
fi
# Prefer SSH if present; fall back to HTTPS
if ! git ls-remote --heads origin >/dev/null 2>&1; then
  git remote set-url origin https://github.com/azmykn/RepoHarbor.git
fi

git push -u origin main --force

# Delete obsolete feature branch on the OLD Orrery fork (safe; PR already merged)
if git remote get-url orrery-old >/dev/null 2>&1; then
  git push orrery-old --delete feat/digitscode-roadmap-p0-p3 2>/dev/null || true
fi

echo
echo "Done."
echo "  Repo:    https://github.com/azmykn/RepoHarbor"
echo "  Commits: $(git rev-list --count HEAD) (expect 1)"
git log --oneline
git remote -v
