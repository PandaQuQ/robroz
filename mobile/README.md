# robroz-mobile (v0.2 spike)

Native Rust client for phones, built with [Slint](https://slint.dev). It reuses
`robroz-contract` and `robroz-card`, and `robroz-ui` without its `gui` (iced)
feature, so the strings come from the same i18n keys as the desktop client.

What it does: connect screen (host URL and token, sent as
`Authorization: Bearer <token>` to `/api`), chat list, one chat with text
bubbles and status cards, and a Settings page (language, About with the Slint
attribution). The connect labels, the unencrypted-HTTP hint, and the 401
token-rejected line use the same `robroz-ui` keys as the desktop client
(`ConnHostLabel`, `ConnTokenLabel`, `ConnConnect`, `ConnUnencryptedHint`,
`ClientTokenRejected`).

Build (see `.github/workflows/mobile.yml` for the exact CI steps):

```sh
# Android APK (needs the Android SDK/NDK, JDK 17, `cargo install cargo-apk`)
cargo apk build --release --lib -p robroz-mobile

# iOS, unsigned .ipa: macOS + Xcode + XcodeGen; see mobile/ios/project.yml
# Desktop, for quick UI work
cargo run -p robroz-mobile
```

`ROBROZ_HOST`, `ROBROZ_TOKEN`, `ROBROZ_LANG` and `ROBROZ_THEME=light|dark`
prefill the desktop build; they are not read on a phone. A saved token is
shown as eight bullets (`••••••••`), same as the desktop form. Never commit
a real token.
