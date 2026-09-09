#!/usr/bin/env bash
#
# Fetch the freely-downloadable story fixtures the test suites need (SQ-1015).
#
# `stories/` is gitignored commercial game media, so every suite that opens a
# file from it skips vacuously on CI — the run that guards a merge. A survey
# (docs/internals/ci-fixture-coverage.md) found that a large minority of those
# suites depend only on works the IF Archive distributes freely. This script
# fetches exactly those, into the directory `fixture_paths::fixture_path`
# already falls back to.
#
# It does NOT vendor them. The repository carries the manifest — a URL, a
# SHA-256 and a byte count per file — and nothing else; the bytes come from the
# upstream that has permission to serve them, every time. See the manifest's own
# header for why that distinction is the whole design.
#
#   scripts/fetch-fixtures.sh                 # fetch + verify into the default dir
#   scripts/fetch-fixtures.sh --dest DIR      # ...into DIR instead
#   scripts/fetch-fixtures.sh --verify-only   # no network; just check what is there
#
# Exits non-zero on ANY missing file or digest mismatch. That is the point: a
# fixture that changed under us is a worse outcome than one that is absent, and
# a fetch that half-failed must not read as a green run full of quiet skips.
#
# Portability: bash, curl, and one of {sha256sum, shasum, openssl} + one of
# {unzip, python3, python}. All present on the three GitHub runners and on a
# developer's macOS/Linux box; Git-bash covers the Windows one.

set -u

usage() {
    sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

repo_root() {
    # The script lives in <root>/scripts/, and must work when invoked by path
    # from anywhere — CI calls it as `scripts/fetch-fixtures.sh` from the root,
    # a developer may not.
    local here
    here=$(cd -- "$(dirname -- "$0")" && pwd)
    dirname -- "$here"
}

ROOT=$(repo_root)
MANIFEST="$ROOT/scripts/fixtures.manifest"
DEST="$ROOT/crates/app/tests/fixtures/stories"
VERIFY_ONLY=0

while [ $# -gt 0 ]; do
    case "$1" in
        --dest) DEST="${2:?--dest needs a directory}"; shift 2 ;;
        --manifest) MANIFEST="${2:?--manifest needs a file}"; shift 2 ;;
        --verify-only) VERIFY_ONLY=1; shift ;;
        -h|--help) usage 0 ;;
        *) echo "unknown argument: $1" >&2; usage 2 ;;
    esac
done

[ -f "$MANIFEST" ] || { echo "no manifest at $MANIFEST" >&2; exit 2; }

# ---------------------------------------------------------------- primitives

# sha256 of a file, lowercase hex, nothing else on the line. Three spellings
# because no one of them is on all three runners: macOS has `shasum` and not
# `sha256sum`, most Linuxes the reverse, Git-bash ships both plus openssl.
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -- "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -- "$1" | cut -d' ' -f1
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 -- "$1" | awk '{print $NF}'
    else
        echo "no sha256 tool (need one of sha256sum, shasum, openssl)" >&2
        exit 3
    fi
}

size_of() {
    # `wc -c` rather than stat(1), whose flags differ between BSD and GNU.
    wc -c < "$1" | tr -d '[:space:]'
}

# Extract one member of a zip to a path. `unzip -p` where there is an unzip,
# Python's zipfile where there is not — Windows runners have Python and may not
# have unzip, and bsdtar's zip support is not on GNU tar.
extract_member() {
    local zip="$1" member="$2" out="$3"
    if command -v unzip >/dev/null 2>&1; then
        unzip -p -- "$zip" "$member" > "$out" 2>/dev/null && [ -s "$out" ] && return 0
    fi
    local py=""
    command -v python3 >/dev/null 2>&1 && py=python3
    [ -z "$py" ] && command -v python >/dev/null 2>&1 && py=python
    if [ -n "$py" ]; then
        "$py" -c 'import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z, open(sys.argv[3], "wb") as o:
    o.write(z.read(sys.argv[2]))' "$zip" "$member" "$out" && return 0
    fi
    echo "cannot extract from a zip (need one of unzip, python3, python)" >&2
    return 1
}

# ------------------------------------------------------------------ the work

mkdir -p -- "$DEST" || exit 3
CACHE="$DEST/.archive-cache"
[ "$VERIFY_ONLY" -eq 1 ] || mkdir -p -- "$CACHE" || exit 3

fetched=0
verified=0
failed=0
missing=0

# Download a URL once per run, into the cache, keyed by a flattened name. The
# zips carry several fixtures each, and re-downloading 19 MB of Kerkerkruip
# once per member would be absurd.
download() {
    local url="$1" key="$2"
    local path="$CACHE/$key"
    if [ -f "$path" ]; then
        echo "$path"
        return 0
    fi
    if ! curl -fsSL --retry 3 --retry-delay 2 --max-time 600 -o "$path.part" -- "$url"; then
        rm -f -- "$path.part"
        return 1
    fi
    mv -- "$path.part" "$path"
    echo "$path"
}

while IFS=$'\t' read -r sha size dest url member licence; do
    case "$sha" in ''|'#'*) continue ;; esac

    target="$DEST/$dest"

    # Already correct? Say nothing and move on — this is the common case on a
    # warm cache and on a developer's second run.
    if [ -f "$target" ] && [ "$(size_of "$target")" = "$size" ] \
       && [ "$(sha256_of "$target")" = "$sha" ]; then
        verified=$((verified + 1))
        continue
    fi

    if [ "$VERIFY_ONLY" -eq 1 ]; then
        if [ -f "$target" ]; then
            echo "MISMATCH  $dest — have sha256 $(sha256_of "$target") ($(size_of "$target") bytes), want $sha ($size)" >&2
            failed=$((failed + 1))
        else
            echo "ABSENT    $dest" >&2
            missing=$((missing + 1))
        fi
        continue
    fi

    # Cache key: the URL's basename is not unique enough on its own (two
    # `mysterious*.zip` live in different archive directories), so flatten the
    # whole path.
    key=$(printf '%s' "$url" | tr -c 'A-Za-z0-9._-' '_')
    if ! src=$(download "$url" "$key"); then
        echo "FETCH FAILED  $dest — $url" >&2
        failed=$((failed + 1))
        continue
    fi

    if [ "$member" = "-" ]; then
        cp -- "$src" "$target.part" || { failed=$((failed + 1)); continue; }
    else
        if ! extract_member "$src" "$member" "$target.part"; then
            echo "EXTRACT FAILED  $dest — $member in $url" >&2
            rm -f -- "$target.part"
            failed=$((failed + 1))
            continue
        fi
    fi

    got_size=$(size_of "$target.part")
    got_sha=$(sha256_of "$target.part")
    if [ "$got_size" != "$size" ] || [ "$got_sha" != "$sha" ]; then
        echo "DIGEST MISMATCH  $dest" >&2
        echo "    want  $sha  $size bytes" >&2
        echo "    got   $got_sha  $got_size bytes" >&2
        echo "    from  $url${member:+ [$member]}" >&2
        rm -f -- "$target.part"
        failed=$((failed + 1))
        continue
    fi
    mv -- "$target.part" "$target"
    echo "fetched  $dest  ($size bytes)  [$licence]"
    fetched=$((fetched + 1))
done < "$MANIFEST"

if [ "$VERIFY_ONLY" -eq 1 ]; then
    echo "verified $verified, absent $missing, mismatched $failed"
    [ $((missing + failed)) -eq 0 ] || exit 1
else
    # The cache is pure re-fetch avoidance; nothing reads it and it is bulky.
    rm -rf -- "$CACHE"
    echo "fetched $fetched, already present $verified, failed $failed"
    [ "$failed" -eq 0 ] || exit 1
fi
