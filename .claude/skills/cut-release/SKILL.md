---
name: cut-release
description: Cut an antfarm release - version bump PR, tag after merge, watch the signed and notarized release workflow, then verify the Homebrew install with Gatekeeper checks. Use when the user asks to cut, ship, or publish a release, or a version such as 0.1.2.
---

# Cut an antfarm release

[docs/RELEASING.md](../../../docs/RELEASING.md) is the reference; this skill runs its "Cutting a release" steps. Only cut a release when the user asks. Never handle secret values; if a secret is missing or expired, tell the user which one, and point to the RELEASING.md section that creates it.

## 1. Choose the version

- Read the current one: `grep -n '^version' Cargo.toml`.
- List what's unreleased: `git log --oneline v<current>..main --merges`.
- Default to the next patch version. Suggest a minor bump when a merged PR added a feature a user would notice, and ask with AskUserQuestion when that's the case.
- Never reuse a version that was ever tagged.

## 2. Bump through a PR

```sh
git switch main && git pull --ff-only
git switch -c release-<version>
sed -i '' 's/^version = "<current>"/version = "<version>"/' Cargo.toml
~/.cargo/bin/cargo check --workspace
git commit -am "Release <version>"
git push -u origin release-<version>
```

The diff must be `Cargo.toml` and `Cargo.lock` only. Open a PR titled "Release <version>" whose body lists each merged PR since the last tag in one plain line, and says the merge commit gets tagged. No attribution lines. Then wait for the user to say "merged".

## 3. Tag and watch

```sh
git switch main && git pull --ff-only && git branch -D release-<version>
grep -n '^version' Cargo.toml   # must show the new version
git tag -a v<version> -m "antfarm <version>"
git push origin v<version>
```

Find the run (`gh run list --workflow release.yml --limit 1 --json databaseId,status`) and wait on it in the background with `gh run watch <id> --exit-status`. Notarization takes several minutes. Tell the user the run id while it runs.

If it fails, read the failing step's log (`gh run view <id> --log-failed`) and follow "If a release fails" in RELEASING.md. Moving a tag or deleting a release is outward-facing, so confirm with the user first.

## 4. Verify

```sh
gh release view v<version> --json assets --jq '.assets[].name'   # dmg, tar.gz, SHA256SUMS
gh api repos/2Toad/homebrew-tap/commits --jq '.[0].commit.message'  # "antfarm <version>"
zsh -lic 'brew update -q; brew upgrade --cask 2toad/tap/antfarm; brew upgrade --formula 2toad/tap/antfarm'
antfarm --version
spctl --assess --type execute --verbose=2 /Applications/antfarm.app
xcrun stapler validate /Applications/antfarm.app
codesign --display --verbose=2 "$(brew --prefix)/bin/antfarm"
defaults read /Applications/antfarm.app/Contents/Info CFBundleShortVersionString
```

Use `brew install` instead of `upgrade` if antfarm isn't installed yet. Expect `accepted`, `source=Notarized Developer ID`, a passing stapler check, and `Authority=Developer ID Application: 2Toad, LLC (DVS62LTJ79)` with the `runtime` flag.

## 5. Report

Tell the user the release is out, with each check's result. Say that opening the app from Finder with no Gatekeeper prompt is the one check left to them, and that a running copy needs a restart to pick up the new version. Update the project memory with the version and date.
