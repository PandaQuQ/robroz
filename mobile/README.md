# robroz-mobile (v0.2 spike)

Native Rust client for phones, built with [Slint](https://slint.dev). It reuses
`robroz-contract` and `robroz-card`, and `robroz-ui` without its `gui` (iced)
feature, so the strings come from the same i18n keys as the desktop client.

What it does: connect screen (host URL and token, sent as
`Authorization: Bearer <token>` to `/api`), chat list, one chat with text
bubbles and status cards, and a Settings page (language, About with the Slint
attribution).

Build (see `.github/workflows/mobile.yml` for the exact CI steps):

```sh
# Android APK (needs the Android SDK/NDK, JDK 17, `cargo install cargo-apk`)
cargo apk build --release --lib -p robroz-mobile

# iOS, unsigned .ipa: macOS + Xcode + XcodeGen; see mobile/ios/project.yml
# Desktop, for quick UI work
cargo run -p robroz-mobile
```

`ROBROZ_HOST`, `ROBROZ_TOKEN` and `ROBROZ_LANG` prefill the desktop build; they
are not read on a phone. Never commit a real token.
