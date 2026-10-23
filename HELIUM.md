# Obelisk (Helium)

Helium's build of [Zagrios's Obelisk](https://github.com/Zagrios/Obelisk), from [BrandonSigil/Obelisk](https://github.com/BrandonSigil/Obelisk). It is GPL-3.0 like Obelisk, and its source is this repository.

## How it differs

- **Its own app**: shown as *Obelisk (Helium)*, bundle identifier `com.heliumdev.obelisk` (`apps/macos/Config/App.xcconfig`). It shares Obelisk's library, `~/Library/Application Support/com.zagrios.obelisk`, so games and sign-ins stay where they are. Never run both apps at once.
- **Its own updates**: Sparkle reads the `appcast.xml` of this repository's latest release, signed with Helium's key (`SUFeedURL` and `SUPublicEDKey` in `apps/macos/Config/Info.plist`). Zagrios's releases never replace it.
- **Its own game profiles**: Release builds read `crates/game-runtime/profiles.json` from this repository's `main` as a Windows game launches (`crates/game-runtime/src/profile.rs`).

Voile, Obelisk's Wine runtime, still comes from Zagrios's feed.

## Following Zagrios

`upstream` is Zagrios's repository. To bring in what he released:

```sh
git fetch upstream
git switch main
git merge upstream/main
git push origin main
```

Changes meant for Zagrios go on branches from `upstream/main`, as pull requests to his repository; once he merges them, the next merge brings them back.

## Publishing a version

1. Raise `MARKETING_VERSION` in `apps/macos/Config/App.xcconfig` above the last Helium release, commit, and push a tag `v<version>`. The Release workflow builds, notarizes and drafts the release.
2. Publish the release. The Appcast workflow signs the DMG and attaches `appcast.xml`, which every installed Obelisk (Helium) reads.

The workflows need these repository secrets: `DEVELOPER_ID_P12` and `DEVELOPER_ID_P12_PASSWORD` (a Developer ID Application certificate), `NOTARY_API_KEY`, `NOTARY_API_KEY_ID` and `NOTARY_API_ISSUER_ID` (an App Store Connect API key), and `SPARKLE_ED_PRIVATE_KEY`, Helium's Sparkle key. `generate_keys --account heliumdev -x <file>` exports that key from the login Keychain. Keep a copy of it outside this Mac: without it, installed copies accept no new version.
