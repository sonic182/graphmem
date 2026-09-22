---
name: new-release
description: Prepare a Graphmem release or version bump. Use whenever the user asks to release, cut a version, bump a patch/minor/major version, update the changelog, or synchronize Graphmem, OpenCode/pi, Claude Code, and Codex plugin versions.
compatibility: Requires git, Cargo, just, and a Graphmem checkout.
---

# New Graphmem release

Prepare a consistent release change. Do not commit, tag, push, publish, or create a GitHub release unless the user explicitly asks.

## 1. Establish the target version

1. Inspect `git status --short`; do not overwrite unrelated changes.
2. Read the current version from `Cargo.toml`, `package.json`, and each first-party manifest below. They should agree.
3. If the user names a version, validate it as SemVer. If they ask for a patch, minor, or major bump, calculate the next version from the current one. Ask only when the requested bump is ambiguous.
4. Use `date +%F` for the release date.

Release version declarations:

- `Cargo.toml` — `graphmem` crate
- `Cargo.lock` — generated package entry; let Cargo update it
- `package.json` — OpenCode and pi package
- `plugin/.claude-plugin/plugin.json` — Claude Code plugin
- `plugin/.codex-plugin/plugin.json` — Codex plugin

## 2. Update the release files

1. Set the target version in `Cargo.toml`, `package.json`, and both plugin manifests.
2. In `CHANGELOG.md`, retain an empty `## [Unreleased]` section and move its completed entries under `## [<version>] - <date>`.
3. Add the release link and move the `[unreleased]` comparison link to `<version>...HEAD`:

   ```markdown
   [unreleased]: https://github.com/sonic182/graphmem/compare/<version>...HEAD
   [<version>]: https://github.com/sonic182/graphmem/releases/tag/<version>
   ```

4. Run `just check` so Cargo regenerates `Cargo.lock` with the new crate version. Do not hand-edit dependency lock entries.

## 3. Verify

Run:

```sh
just fmt
just verify
git diff --check
```

Confirm the first-party release declarations all contain the target version and that the `graphmem` package entry in `Cargo.lock` does too. Review `git diff --stat` and `git diff` before handoff.

## 4. Handoff

Report the target version, updated paths, and verification result. State separately whether the release is only prepared or was also committed, tagged, pushed, or published.

If asked to tag, use Graphmem's bare version convention (for example `0.3.1`, not `v0.3.1`) and annotate it with `release version <version>`.
