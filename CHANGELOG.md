# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- The Linux client's X11 window icon (`_NET_WM_ICON`) is now 128×128 instead of 512×512, so window managers and taskbars that use it show the robroz icon instead of a generic one. The 512×512 property was about 1 MiB, larger than many X11 clients read ([#167](https://github.com/PandaQuQ/robroz/pull/167)).
- The Windows client no longer opens an extra command-line window on start ([#165](https://github.com/PandaQuQ/robroz/pull/165)). Release builds use the Windows GUI subsystem. Debug builds keep the console. When the client is started from Explorer, stderr output such as a failed preference save or a panic message is not shown; start it from a shell to see it.

## [0.1.0] - 2026-11-25

Published as a pre-release. First release of robroz: a single-user, self-hosted multi-agent workspace with a Rust host and a native desktop client.

### Added

- Window icon for the desktop client, and a Windows exe icon ([#150](https://github.com/PandaQuQ/robroz/pull/150)).
- App icons in release packages ([#161](https://github.com/PandaQuQ/robroz/pull/161)): `robroz.app` ships `icon.icns` (`CFBundleIconFile`); the Linux client tarball ships `robroz.desktop` and `robroz.png`. The same PR adds a `workflow_dispatch` dry run: it builds and packages the binaries, and builds the runner image without pushing it or logging in to GHCR. It does not create a release.
- Release packaging when a `v*` tag is pushed ([#149](https://github.com/PandaQuQ/robroz/pull/149)): draft GitHub Release assets, unsigned, with `SHA256SUMS.txt`. Client archives are Linux x86_64 (`.tar.gz`), Windows x86_64 (`.zip` with the `.exe`), and macOS aarch64 only (`.zip` containing `robroz.app`). The host ships for Linux x86_64 only. Intel Macs are not covered.
- Runner image `ghcr.io/pandaquq/robroz-runner:<tag>` on `v*` tags ([#151](https://github.com/PandaQuQ/robroz/pull/151)): `linux/amd64` only, tagged with the git tag, no `latest` tag. Example: `ROBROZ_IMAGE=ghcr.io/pandaquq/robroz-runner:v0.1.0`.
- Manual platform screenshot workflow for Windows and macOS ([#153](https://github.com/PandaQuQ/robroz/pull/153), [#154](https://github.com/PandaQuQ/robroz/pull/154)).
- Client HTTPS via rustls, and persisted connection settings ([#152](https://github.com/PandaQuQ/robroz/pull/152)). The host address is stored in a `host` file next to `lang`. The token is stored in the OS keyring (macOS Keychain, Windows Credential Manager, Secret Service on Linux), falling back to a `token` file next to `lang` with mode `0600` on Unix. `ROBROZ_HOST` and `ROBROZ_TOKEN` override saved values for the current run and are never written back.
- In-app connection form with Host, Token, and Connect, on the offline pane and in Settings ([#159](https://github.com/PandaQuQ/robroz/pull/159)). A field set by the environment is read-only (`Set by ROBROZ_HOST` / `Set by ROBROZ_TOKEN`). When both are set by the environment, Connect is disabled. A non-local `http://` host shows `Unencrypted. Use HTTPS for remote hosts.` A 401 is titled `Token rejected` (「令牌被拒绝」), with the reason `The host rejected the token. Check ROBROZ_TOKEN.` An unreachable host is titled `Can't reach the host` (「连不上宿主」); the reason line is `Can't reach host.` (「连不上宿主。」) plus the detail.
- Chat feed rendered as bubbles ([#155](https://github.com/PandaQuQ/robroz/pull/155)). Your messages sit on the right; bro status, approval, choice, and confirm cards sit on the left. A delivery with `bot_name` is attached to that bro and follows the usual 2-minute grouping; a delivery without `bot_name` stands alone, with no avatar or name. On macOS the UI font is Hiragino Sans GB; on Windows it is Microsoft YaHei UI.
- Optional `taskId`, `botId`, and `botName` on `Event::Delivery` ([#158](https://github.com/PandaQuQ/robroz/pull/158)). The fields are omitted when absent, so older JSON serializes unchanged.
- Appearance setting ([#160](https://github.com/PandaQuQ/robroz/pull/160)): a Settings row Appearance / 「外观」 under Language, also on the Can't reach the host page, with System / 「跟随系统」, Light / 「浅色」, and Dark / 「深色」. The default is System, which follows the OS live (`iced::system::theme_changes()`). The choice is saved as one line (`system`, `light`, or `dark`) in a `theme` file next to `lang`; no file means System. The footer Dark toggle is removed. `ROBROZ_THEME=light|dark` is honoured only in fake and gallery runs; a fake run with no variable defaults to Light.

### Fixed

- Dark-mode segmented controls show the selected option more clearly ([#162](https://github.com/PandaQuQ/robroz/pull/162)).
- Linux release builds run on older distributions ([#164](https://github.com/PandaQuQ/robroz/pull/164)). They are built on Ubuntu 22.04, and the release job fails if a binary needs glibc newer than 2.35. The client needs glibc 2.35 or newer; the host needs 2.34 or newer. The Linux icon is renamed to `robroz.png` to match `Icon=robroz` in `robroz.desktop`.

### Known issues

- Windows: an extra command-line window opens when the client starts. Leave it open; closing it also quits the client. Fixed in the next release.

[Unreleased]: https://github.com/PandaQuQ/robroz/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/PandaQuQ/robroz/releases/tag/v0.1.0
