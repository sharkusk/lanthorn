# Releasing

[← back to README](../../README.md)

A hand-run procedure, not a script — cutting a release touches two workflows,
a commit that has to read well as a GitHub release body, and one manual step
on a package registry that only exists the first time. This page is the
checklist; the reasoning for each piece lives in the workflow files and in
`README.md`/`CHANGELOG.md`'s own top-of-file notes.

## 1. Preconditions

- `main` is green in Actions.
- Every quest the release depends on that sits at `confirm` has been looked
  at — `confirm` means only a person's eye could settle it (rendering, feel,
  audio, a real-game smoke), so it is the one status a green CI run cannot
  vouch for on its own.
- `cargo build --release --workspace`, then smoke one story per engine on
  the **release** binary. Overflow bugs behave differently in release than
  under the debug build the test gate runs, so a clean `cargo nextest run`
  does not by itself prove the release binary boots a game.

## 2. Dry-run both workflows first

Actions → **Release** → Run workflow, and Actions → **Docker** → Run
workflow — or `gh workflow run release.yml --ref main` /
`gh workflow run docker.yml --ref main`. Neither dry run needs a tag.

- **Release**'s dry run builds every platform and uploads the archives as
  run artifacts, without creating a release.
- **Docker**'s dry run builds the image without pushing it anywhere.

Both are cheap ways to catch a build failure before it is a public red run
against a tag.

## 3. The release commit

- Drain every `*Next release:*` tag from `README.md` — most just lose the
  tag and get their first letter capitalised; a renamed-flag parenthetical
  (`` `--sound off` *(next release; today `--no-sound`)* ``) gets rewritten
  to name only the current spelling, not just have its tag deleted.
- Rename `## Unreleased` in `CHANGELOG.md` to `## vX.Y.Z — YYYY-MM-DD`, with
  a `### Highlights` list as the first thing under it — the handful of
  bullets a player downloading the build cares about most, a breaking change
  first if there is one.
- Bump `version` in the workspace `Cargo.toml`'s `[workspace.package]` —
  every crate and every binary's `--version` follow from that one line.
- Commit, push to `main`, wait for green.

## 4. Tag

```sh
git tag vX.Y.Z && git push origin vX.Y.Z
```

A hyphenated tag (`v0.4.0-rc.1`) publishes as a pre-release and gets no
`latest` image tag — both workflows key off the hyphen the same way.
On a tag push — and only there, so that dry runs on main still reach the
builds — `release.yml`'s guard step refuses to build while a `next release`
tag or an `Unreleased` section survives, so a release commit that skipped
step 3 fails here instead of shipping half-drained docs.

## 5. After the tag push

- The GitHub Release is created as a **draft** — the release body is the
  `CHANGELOG.md` section verbatim. Inspect the attached archives and the
  body, then Publish.
- The **first** push to `ghcr.io/sharkusk/lanthorn` creates a **private**
  package. Open the package's settings on GitHub and change visibility to
  Public, or `docker pull ghcr.io/sharkusk/lanthorn:latest` fails for anyone
  not logged in. One-time, but easy to forget — nothing in either workflow
  does it for you.
- If **Docker** failed on the tag while **Release** succeeded, do not re-tag
  (that re-runs the release build too). Run the Docker workflow by hand
  naming the tag — `gh workflow run docker.yml --ref main -f tag=v0.4.0` —
  and it builds that tag and pushes it, exactly as the tag push would have.

## 6. Afterwards

`README.md` now describes the new release, so new unreleased work goes back
to carrying `*Next release:*` tags at each feature's normal spot in the
README. Open a fresh `## Unreleased` section in `CHANGELOG.md` when the
first post-release feature lands, with the same reminder this section
carried right after `## Unreleased` (drained by this release's own commit,
so it won't be at HEAD to copy from):

> *This section is drained when a version is cut. README.md describes the
> RELEASED build; prose for a feature that is in `main` but not yet released
> goes into the README in place, at its normal destination, marked with the
> visible tag `*Next release:*`. `release.yml` refuses to cut a release
> while any such tag, or this Unreleased section, still exists.*
