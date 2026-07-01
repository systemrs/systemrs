#!/usr/bin/env bash
# Check that commit subjects on this branch follow Conventional Commits, so
# release-plz can derive the correct SemVer bump (feat -> minor, fix -> patch,
# `!`/`BREAKING CHANGE:` -> breaking). Merge commits are allowed and skipped.
#
# Set BASE to the ref to compare against (default: origin/master). CI passes the
# pull-request base branch. Locally, `just commit-lint` checks commits since
# origin/master; with no such ref it falls back to the tip commit only.
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

base="${BASE:-origin/master}"
if git rev-parse --verify --quiet "$base" >/dev/null; then
    range="$(git merge-base "$base" HEAD)..HEAD"
else
    printf 'commit-lint: %s not found; checking the tip commit only.\n' "$base" >&2
    range="HEAD~1..HEAD"
fi

# type(optional-scope)!: description   (the `!` and scope are optional)
pattern='^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\([^)]+\))?!?: .+'

fail=0
while IFS= read -r sha; do
    [ -z "$sha" ] && continue
    # Skip merge commits (they legitimately have a non-conventional subject).
    if git rev-parse --verify --quiet "$sha^2" >/dev/null; then
        continue
    fi
    subject="$(git log -1 --format=%s "$sha")"
    if ! printf '%s' "$subject" | grep -Eq "$pattern"; then
        printf 'Non-conventional commit %s: %s\n' "${sha:0:8}" "$subject" >&2
        fail=1
    fi
done < <(git rev-list "$range")

if [ "$fail" -ne 0 ]; then
    cat >&2 <<'MSG'

Commit subjects must follow Conventional Commits, for example:
  feat(kernel): add delta-cycle change_stamp accounting
  fix: correct immediate-notification collapse order
  refactor(tlm2)!: rename GenericPayload::data   (a breaking change: use `!` or a BREAKING CHANGE footer)

See https://www.conventionalcommits.org/. release-plz derives the version bump from these subjects.
MSG
    exit 1
fi
echo "All commit subjects follow Conventional Commits."
