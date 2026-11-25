# robroz-mobile (v0.2 spike)

Native Rust client for phones, built with [Slint](https://slint.dev). It reuses
`robroz-contract` and `robroz-card`, and `robroz-ui` without its `gui` (iced)
feature, so the strings come from the same i18n keys as the desktop client.

What it does: connect screen (host URL and token, sent as
`Authorization: Bearer <token>` to `/api`), chat list, one chat with text
bubbles and status cards, and a Settings page (language, About with the Slint
attribution, and an Appearance row with System / Light / Dark). The connect
labels, the unencrypted-HTTP hint, and a rejected token use the same
`robroz-ui` keys as the desktop client (`ConnHostLabel`, `ConnTokenLabel`,
`ConnConnect`, `ConnUnencryptedHint`, `ClientTokenRejectedTitle`). A 401
shows the title "Token rejected" / 「令牌被拒绝」 and only adds the
`ConnSetByToken` hint when the token really came from `ROBROZ_TOKEN`.

Build (see `.github/workflows/mobile.yml` for the exact CI steps):

```sh
# Android APK (needs the Android SDK/NDK, JDK 17, `cargo install cargo-apk`)
cargo apk build --release --lib -p robroz-mobile

# iOS, unsigned .ipa: macOS + Xcode + XcodeGen; see mobile/ios/project.yml
# Desktop, for quick UI work
cargo run -p robroz-mobile
```

`ROBROZ_HOST`, `ROBROZ_TOKEN`, `ROBROZ_LANG` and `ROBROZ_THEME=system|light|dark`
prefill the desktop build (appearance defaults to System); they are not read on a phone. A saved token is
shown as eight bullets (`••••••••`), same as the desktop form. Never commit
a real token.
