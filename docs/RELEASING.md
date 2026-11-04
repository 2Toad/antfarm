# Releasing antfarm

Releases are built by GitHub Actions when a `v*` tag is pushed. The workflow signs and notarizes the desktop app and the CLI, publishes a GitHub release, and updates the Homebrew tap at [2Toad/homebrew-tap](https://github.com/2Toad/homebrew-tap).

## One-time setup

This needs to happen once, before the first release. Steps 1 and 2 need the Apple Developer team's Account Holder (2Toad, LLC, team id `DVS62LTJ79`). Steps 0 and 3 need an owner of the 2Toad GitHub organization.

Nothing here should be pasted into a chat, an issue, or a commit. The passwords, the `.p12`, and the `.p8` go only into your password manager and into GitHub secrets in step 4.

### 0. Make the antfarm repo public

Homebrew downloads release files without signing in to GitHub, so `brew install` fails against a private repo's releases. The history was scanned with gitleaks on 2026-11-04: its hits are the fake tokens in the redaction tests and scrubbed ids in `fixtures/real`, and the only personal detail is the author email in commits and `Cargo.toml`.

1. Open [antfarm settings](https://github.com/2Toad/antfarm/settings), scroll to the Danger Zone, choose **Change repository visibility**, and make it public. Or, from Terminal:

   ```sh
   gh repo edit 2Toad/antfarm --visibility public --accept-visibility-change-consequences
   ```

### 1. Developer ID Application certificate

This certificate signs apps that are distributed outside the Mac App Store. The Apple Development and Apple Distribution certificates already in the keychain can't be used for this.

1. Create a certificate signing request on this Mac. Apple's guide: [Create a certificate signing request](https://developer.apple.com/help/account/certificates/create-a-certificate-signing-request).
   1. Open Keychain Access.
   2. From the menu bar, choose Keychain Access, then Certificate Assistant, then Request a Certificate From a Certificate Authority.
   3. Enter your email address and `2Toad Developer ID` as the common name, leave the CA email address empty, and choose "Saved to disk".
   4. Save the `.certSigningRequest` file to the Desktop.
2. Go to [Certificates, add a new one](https://developer.apple.com/account/resources/certificates/add) in the Apple Developer portal. Apple's guide: [Create Developer ID certificates](https://developer.apple.com/help/account/certificates/create-developer-id-certificates).
   1. Under Software, choose **Developer ID Application**, then Continue.
   2. Choose the **G2 Sub-CA** profile type.
   3. Upload the signing request from step 1, then Continue.
   4. Download the `.cer` file and double-click it. It installs into the login keychain, next to the private key that step 1 created.
3. Check that it installed. In Terminal, run:

   ```sh
   security find-identity -v -p codesigning
   ```

   The output should include this line:

   ```
   "Developer ID Application: 2Toad, LLC (DVS62LTJ79)"
   ```

4. Export it with its private key.
   1. In Keychain Access, select the **login** keychain and the **My Certificates** tab.
   2. Find "Developer ID Application: 2Toad, LLC". Expand it and check that a private key is underneath. Without one, the export can't sign anything.
   3. Right-click the certificate, choose Export, choose the Personal Information Exchange (`.p12`) format, and save it as `~/Desktop/antfarm-developer-id.p12`.
   4. Set a strong password when asked, and save the password in your password manager.
5. Delete the `.certSigningRequest` file. It isn't needed again.

### 2. App Store Connect API key for notarization

Notarization sends each build to Apple to be checked. The workflow signs in with an API key rather than an Apple ID password.

1. Open [App Store Connect, Users and Access, Integrations](https://appstoreconnect.apple.com/access/integrations/api) and select **Team Keys**. Apple's guide: [Creating API keys for App Store Connect API](https://developer.apple.com/documentation/appstoreconnectapi/creating-api-keys-for-app-store-connect-api).
2. If this is the team's first API key, click **Request Access**, accept the terms, and reload the page.
3. Click **Generate API Key**, or the **+** button if keys already exist. Name it `antfarm notarization` and choose the **Developer** access role.
4. Download the key. The file is named like `AuthKey_ABC123DEFG.p8`, and **Apple lets you download it only once**. Save a copy in your password manager.
5. Note two values from the page:
   - **Key ID**, in the key's row, for example `ABC123DEFG`.
   - **Issuer ID**, the UUID shown above the list of keys.
6. Store the key for local notarization too. A local run of `scripts/release-build.sh` uses this profile, named `antfarm`, which keeps the key in your keychain:

   ```sh
   xcrun notarytool store-credentials antfarm \
     --key ~/Downloads/AuthKey_ABC123DEFG.p8 \
     --key-id ABC123DEFG \
     --issuer 00000000-0000-0000-0000-000000000000
   ```

   It should end with "Credentials saved to Keychain".

### 3. The Homebrew tap repo and its token

The release workflow pushes updated cask and formula files to the tap. It needs a token that can write to that one repo and nothing else.

1. Create the tap repo. It must be public and named `homebrew-tap`, so that `brew install 2toad/tap/...` finds it:

   ```sh
   gh repo create 2Toad/homebrew-tap --public --add-readme --description "Homebrew formulae and casks for 2Toad tools"
   ```

2. Check that the organization allows fine-grained tokens. In [2Toad settings, Personal access tokens](https://github.com/organizations/2Toad/settings/personal-access-tokens), allow access via fine-grained personal access tokens. Approval can be required or not; requiring it adds step 4 below.
3. Create the token at [New fine-grained personal access token](https://github.com/settings/personal-access-tokens/new). GitHub's guide: [Creating a fine-grained personal access token](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#creating-a-fine-grained-personal-access-token).
   - **Token name:** `antfarm release: homebrew-tap`
   - **Resource owner:** 2Toad
   - **Expiration:** one year. Add a calendar reminder to rotate it.
   - **Repository access:** Only select repositories, then `2Toad/homebrew-tap`.
   - **Permissions:** Repository permissions, then **Contents: Read and write**. Metadata: Read-only is added automatically.
4. If the organization requires approval, approve the request in [2Toad settings, Pending requests](https://github.com/organizations/2Toad/settings/personal-access-token-requests).
5. Copy the token when GitHub shows it, which happens only once.

### 4. Store the secrets in the antfarm repo

Run these from the repo in Terminal. Commands without `--body` prompt for the value, so passwords and tokens never land in shell history. Replace the example key id, issuer id, and file names with yours.

```sh
cd ~/2toad/projects/antfarm

# The certificate from step 1, as base64, and its export password.
base64 -i ~/Desktop/antfarm-developer-id.p12 | gh secret set APPLE_CERTIFICATE
gh secret set APPLE_CERTIFICATE_PASSWORD
gh secret set APPLE_SIGNING_IDENTITY --body "Developer ID Application: 2Toad, LLC (DVS62LTJ79)"

# The API key from step 2.
gh secret set APPLE_API_KEY --body "ABC123DEFG"
gh secret set APPLE_API_ISSUER --body "00000000-0000-0000-0000-000000000000"
gh secret set APPLE_API_KEY_P8 < ~/Downloads/AuthKey_ABC123DEFG.p8

# The tap token from step 3.
gh secret set TAP_TOKEN
```

Check that all seven are there:

```sh
gh secret list
```

It should list `APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_P8`, `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, and `TAP_TOKEN`.

Once your password manager has copies, delete `~/Desktop/antfarm-developer-id.p12` and the `.p8` in Downloads. The certificate and private key stay in your login keychain for local signing.

## Cutting a release

The version lives in one place, `version` under `[workspace.package]` in the root `Cargo.toml`. The CLI, the app, and the Homebrew files all take it from there.

1. Bump the version on a branch, refresh the lockfile, and merge it through a pull request like any other change:

   ```sh
   # edit Cargo.toml: version = "0.2.0"
   cargo check --workspace
   git commit -am "Release 0.2.0"
   ```

2. Optionally, do a dry run on your Mac first. It runs exactly what CI runs, including notarization, using the `antfarm` keychain profile from step 2 of the setup, and leaves the results in `dist-release/`:

   ```sh
   NOTARY_PROFILE=antfarm scripts/release-build.sh
   ```

   With `SKIP_NOTARIZE=1` instead of `NOTARY_PROFILE`, it only builds and signs, which takes a few minutes rather than waiting on Apple.

3. Tag the merged commit on main and push the tag:

   ```sh
   git switch main && git pull
   git tag -a v0.2.0 -m "antfarm 0.2.0"
   git push origin v0.2.0
   ```

4. Watch the Release workflow. It fails straight away if the tag doesn't match the version in `Cargo.toml`. Notarization usually takes a few minutes per file, and there are three files.

   ```sh
   gh run watch "$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId')"
   ```

5. Check the result from a clean install:

   ```sh
   brew update
   brew install --cask 2toad/tap/antfarm
   brew install 2toad/tap/antfarm
   antfarm --version
   spctl --assess --type execute --verbose=2 /Applications/antfarm.app
   xcrun stapler validate /Applications/antfarm.app
   codesign --display --verbose=2 "$(brew --prefix)/bin/antfarm"
   ```

   `spctl` should say `accepted` and `source=Notarized Developer ID`. Then open the app from Finder: it should start with no Gatekeeper prompt.

### If a release fails

If the workflow fails before "Publish the GitHub release", nothing was published: fix the problem on main, then move the tag to the new commit and push it again:

```sh
git tag -d v0.2.0 && git push origin :refs/tags/v0.2.0
git tag -a v0.2.0 -m "antfarm 0.2.0" && git push origin v0.2.0
```

If it failed after publishing, for example while updating the tap, delete the release with `gh release delete v0.2.0` first, then retag the same way. Don't reuse a version that Homebrew users may already have installed; bump to the next patch version instead.

### Rotating secrets

- **`TAP_TOKEN`** expires after a year. Create a new token as in step 3 of the setup and run `gh secret set TAP_TOKEN`.
- **The Developer ID certificate** is valid for five years. Releases already published stay valid after it expires, because each signature is timestamped. Renew it as in step 1, then update `APPLE_CERTIFICATE` and `APPLE_CERTIFICATE_PASSWORD`.
- **The API key** doesn't expire. If it leaks, revoke it in App Store Connect and repeat step 2.
