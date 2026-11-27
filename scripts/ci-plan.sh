#!/usr/bin/env bash
# Decides which CI jobs a run needs (.github/workflows/ci.yml's `plan` job;
# docs/TESTING.md "CI"). Prints `name=true|false` lines for $GITHUB_OUTPUT:
#
#   engine   engine tests on Linux, Windows and macOS
#   app      the app's headless UI tests
#   linux, web, android, apple, windows   that platform's package
#   checks   tests, e2e and device checks (false: build packages only)
#
# Everything runs on a push to main, on a pull request, and when the run is
# asked for "all". Otherwise, in this order:
#
# 1. A manual run (workflow_dispatch) names its platforms (PLATFORMS: auto,
#    all, android, apple, windows, linux, web, engine, app) and whether to
#    check them (CHECKS: true/false).
# 2. The head commit's message may say `[ci android]`, `[ci apple windows]`,
#    `[ci all]`, `[ci none]`; `build` in it builds without checks
#    (`[ci android build]`). GitHub's own `[skip ci]` skips the run.
# 3. Else the files changed since the branch's last successful run (else
#    the last push, else main) decide (map_files below): docs alone run nothing; engine code
#    runs the engine and app tests; a platform's own code (its bridges, its
#    checks, `cfg(target_os = …)` lines) runs that platform's package and
#    checks; the workflow, the lock file or nsh.json run everything; a path
#    no rule knows runs everything.
#
#   scripts/ci-plan.sh --self-test   checks the rules
set -euo pipefail

AREAS=(engine app linux web android apple windows)

# The areas files need: one path a line on stdin, the changed lines of Rust
# (unified diff) in $DIFF. Prints area words; `all` for everything.
map_files() {
  local f out=()
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    case "$f" in
      # Run everything: what every job is made of.
      .github/workflows/ci.yml | Cargo.lock | Cargo.toml | rust-toolchain* | \
        app/nsh.json | app/Cargo.toml | app/e2e/* | scripts/ci-plan.sh)
        out+=(all) ;;
      # Nothing to build.
      docs/* | *.md | LICENSE* | .claude/* | .gitignore | .github/workflows/native-sim.yml) ;;
      # A platform's own code and checks.
      crates/dexdrop-net/android/* | crates/dexdrop-net/src/android_service.rs | \
        crates/dexdrop-net/src/jvm.rs | crates/dexdrop-net/src/apps.rs | \
        crates/dexdrop-net/src/assets.rs | crates/dexdrop-net/src/discovery/nsd.rs | \
        crates/dexdrop-net/Cargo.toml | app/src/android.rs | \
        scripts/android-* | scripts/build-nsd-dex.sh)
        out+=(android engine app) ;;
      crates/dexdrop-photos/*)
        out+=(apple app) ;;
      crates/dexdrop-net/src/discovery/bonjour.rs)
        out+=(apple engine) ;;
      scripts/ios-sim-check.sh | scripts/macos-ui-check.sh)
        out+=(apple) ;;
      scripts/desktop-check.py)
        out+=(linux windows apple) ;;
      # Shared code: its tests; platforms only for their own lines (below).
      crates/dexdrop-core/* | crates/dexdrop-net/* | crates/dexdrop-cli/* | crates/dexdrop-bench/*)
        out+=(engine app) ;;
      app/src/* | app/tests/* | app/assets/* | app/theme.json | app/*.png)
        out+=(app) ;;
      *)
        out+=(all) ;;
    esac
  done
  # Lines of a platform's own (`cfg(target_os = …)`) in the Rust changed.
  local d="${DIFF:-}"
  grep -q 'target_os = "android"' <<<"$d" && out+=(android)
  grep -qE 'target_os = "(ios|macos)"|target_vendor = "apple"' <<<"$d" && out+=(apple)
  grep -q 'target_os = "windows"' <<<"$d" && out+=(windows)
  grep -q 'target_os = "linux"' <<<"$d" && out+=(linux)
  grep -q 'target_arch = "wasm32"' <<<"$d" && out+=(web)
  printf '%s\n' "${out[@]}" | sort -u | tr '\n' ' '
}

# Area words (`android apple`, `all`, `ios`) as the areas to run.
normalize() {
  local w out=()
  for w in $(tr ',' ' ' <<<"$*"); do
    case "$w" in
      all) out+=("${AREAS[@]}") ;;
      ios | macos | mac | iphone) out+=(apple) ;;
      engine | app | linux | web | android | apple | windows) out+=("$w") ;;
    esac
  done
  printf '%s\n' "${out[@]}" | sort -u | tr '\n' ' '
}

# Prints the outputs for the areas given (and `checks`).
emit() {
  local chosen=" $1 " checks=$2 a
  for a in "${AREAS[@]}"; do
    if [[ "$chosen" == *" $a "* ]]; then echo "$a=true"; else echo "$a=false"; fi
  done
  echo "checks=$checks"
}

plan() {
  local event=${EVENT:-push} ref=${REF:-} message=${MESSAGE:-}
  local platforms=${PLATFORMS:-auto} checks=${CHECKS:-true}
  if [ "$event" = workflow_dispatch ] && [ "$platforms" != auto ]; then
    echo "reason=asked for: $platforms" >&2
    emit "$(normalize "$platforms")" "$checks"
    return
  fi
  if [ "$event" = pull_request ] || [ "$ref" = refs/heads/main ]; then
    echo "reason=$event to ${BASE_REF:-$ref}: everything" >&2
    emit "$(normalize all)" true
    return
  fi
  # A tag counts in the subject line or on a line of its own, not quoted in
  # the body (a message describing the tags is not asking for one).
  local tag
  tag=$( { head -1 <<<"$message"; grep -E '^[[:space:]]*\[ci [a-z ,-]+\][[:space:]]*$' <<<"$message" || true; } |
    grep -oE '\[ci [a-z ,-]+\]' | head -1 | sed -E 's/^\[ci (.*)\]$/\1/' || true)
  if [ -n "${TAG_ONLY:-}" ]; then # the self-test: the tag found, if any
    echo "${tag:-none}"
    return
  fi
  if [ -n "$tag" ] && [ "$tag" != skip ]; then
    [[ " $tag " == *" build "* ]] && checks=false
    echo "reason=commit says [ci $tag]" >&2
    emit "$(normalize "$tag")" "$checks"
    return
  fi
  # The files changed: since the last push, or since main for a new branch
  # (or a push whose last commit is gone).
  # Since the branch's last successful run (LAST_GREEN): a push whose run
  # was cancelled by the next one (CI run 106) has its changes checked by
  # the next. Else since the last push; else since main.
  local base=""
  if [ -n "${LAST_GREEN:-}" ] && git cat-file -e "$LAST_GREEN^{commit}" 2>/dev/null &&
    git merge-base --is-ancestor "$LAST_GREEN" HEAD 2>/dev/null; then
    base=$LAST_GREEN
  else
    base=${BEFORE:-}
  fi
  if [ -z "$base" ] || [ "$base" = 0000000000000000000000000000000000000000 ] ||
    ! git cat-file -e "$base^{commit}" 2>/dev/null; then
    base=$(git merge-base origin/main HEAD 2>/dev/null || true)
  fi
  if [ -z "$base" ]; then
    echo "reason=no base to compare with: everything" >&2
    emit "$(normalize all)" true
    return
  fi
  local files diff areas
  files=$(git diff --name-only "$base" HEAD)
  diff=$(git diff -U8 "$base" HEAD -- '*.rs' | grep -E '^[-+ ]' || true)
  areas=$(DIFF="$diff" map_files <<<"$files")
  echo "reason=changed since ${base:0:7}: $(grep -c . <<<"$files" || true) files -> ${areas:-nothing}" >&2
  emit "$(normalize "$areas")" "$checks"
}

self_test() {
  local failed=0
  expect() { # expect <files (comma-separated)> <diff> <areas>
    local got
    got=$(DIFF="$2" map_files <<<"$(tr ',' '\n' <<<"$1")")
    got=$(normalize "$got")
    local want
    want=$(normalize "$3")
    if [ "$got" != "$want" ]; then
      echo "FAIL: $1 -> '$got', expected '$want'" >&2
      failed=1
    fi
  }
  expect "docs/TESTING.md,README.md" "" ""
  expect "crates/dexdrop-core/src/manifest.rs" "" "engine app"
  expect "crates/dexdrop-net/android/dev/dexdrop/NsdBridge.java" "" "android engine app"
  expect "crates/dexdrop-photos/src/ios.rs" "" "apple app"
  expect "app/src/pages/send.rs" "" "app"
  expect "app/src/pages/send.rs" '+    #[cfg(target_os = "android")]' "app android"
  expect "app/src/files.rs" ' #[cfg(target_os = "ios")]' "app apple"
  expect "app/src/ui.rs" '-#[cfg(target_arch = "wasm32")]' "app web"
  expect "scripts/ios-sim-check.sh" "" "apple"
  expect "scripts/desktop-check.py" "" "linux windows apple"
  expect ".github/workflows/ci.yml" "" "all"
  expect "Cargo.lock" "" "all"
  expect "app/e2e/app.test.ts" "" "all"
  expect "somewhere/new.txt" "" "all"
  # The outputs: a manual Android build without checks.
  local out
  out=$(EVENT=workflow_dispatch PLATFORMS=android CHECKS=false plan 2>/dev/null)
  grep -qx "android=true" <<<"$out" && grep -qx "apple=false" <<<"$out" &&
    grep -qx "checks=false" <<<"$out" || { echo "FAIL: dispatch android" >&2; failed=1; }
  out=$(EVENT=push REF=refs/heads/main plan 2>/dev/null)
  [ "$(grep -c '=true' <<<"$out")" = 8 ] || { echo "FAIL: main runs everything" >&2; failed=1; }
  out=$(EVENT=push REF=refs/heads/x MESSAGE=$'Fix\n\n[ci apple build]' plan 2>/dev/null)
  grep -qx "apple=true" <<<"$out" && grep -qx "engine=false" <<<"$out" &&
    grep -qx "checks=false" <<<"$out" || { echo "FAIL: [ci apple build]" >&2; failed=1; }
  # A tag quoted in a message's body is not a request (run 101 built only
  # Android for a commit that described `[ci android]`).
  out=$(EVENT=push REF=refs/heads/x BEFORE=none MESSAGE=$'CI: tags\n\n- a tag: [ci android], [ci all]' \
    TAG_ONLY=1 plan 2>/dev/null)
  [ "$out" = none ] || { echo "FAIL: a quoted tag was taken: $out" >&2; failed=1; }
  out=$(EVENT=push REF=refs/heads/x MESSAGE=$'Fix the APK\n\n[ci android]' TAG_ONLY=1 plan 2>/dev/null)
  [ "$out" = android ] || { echo "FAIL: a tag on its own line: $out" >&2; failed=1; }
  out=$(EVENT=push REF=refs/heads/x MESSAGE='Fix the APK [ci android build]' TAG_ONLY=1 plan 2>/dev/null)
  [ "$out" = "android build" ] || { echo "FAIL: a tag in the subject: $out" >&2; failed=1; }
  # The base: the last green run's commit when it is an ancestor, so a
  # cancelled run's changes are not skipped (CI run 106).
  local tmp
  tmp=$(mktemp -d)
  (
    cd "$tmp" && git init -q && git config user.email t@t && git config user.name t &&
      mkdir -p crates/dexdrop-net/android docs &&
      echo a >docs/a.md && git add -A && git commit -qm base &&
      echo b >crates/dexdrop-net/android/B.java && git add -A && git commit -qm android &&
      echo c >docs/c.md && git add -A && git commit -qm docs
  ) >/dev/null
  local green last
  green=$(git -C "$tmp" rev-parse HEAD~2)
  last=$(git -C "$tmp" rev-parse HEAD~1)
  out=$(cd "$tmp" && EVENT=push REF=refs/heads/x BEFORE=$last LAST_GREEN=$green plan 2>/dev/null)
  grep -qx "android=true" <<<"$out" || { echo "FAIL: the last green run is the base" >&2; failed=1; }
  out=$(cd "$tmp" && EVENT=push REF=refs/heads/x BEFORE=$last LAST_GREEN=0123abcd plan 2>/dev/null)
  grep -qx "android=false" <<<"$out" || { echo "FAIL: an unknown green commit falls back to the last push" >&2; failed=1; }
  rm -rf "$tmp"
  [ "$failed" = 0 ] && echo "ci-plan: rules hold"
  return "$failed"
}

if [ "${1:-}" = --self-test ]; then
  self_test
else
  plan
fi
