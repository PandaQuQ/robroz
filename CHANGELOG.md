# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - Unreleased

First release of robroz: a single-user, self-hosted multi-agent workspace with a Rust host and a native desktop client.

### Added

- Window icon for the desktop client, and a Windows exe icon ([#150](https://github.com/PandaQuQ/robroz/pull/150)).
- Release packaging when a `v*` tag is pushed ([#149](https://github.com/PandaQuQ/robroz/pull/149)): draft GitHub Release assets, unsigned, with `SHA256SUMS.txt`. Client archives are Linux x86_64 (`.tar.gz`), Windows x86_64 (`.zip` with the `.exe`), and macOS aarch64 only (`.zip` containing `robroz.app`). The host ships for Linux x86_64 only. Intel Macs are not covered.
- Runner image `ghcr.io/pandaquq/robroz-runner:<tag>` on `v*` tags ([#151](https://github.com/PandaQuQ/robroz/pull/151)): `linux/amd64` only, tagged with the git tag, no `latest` tag. Example: `ROBROZ_IMAGE=ghcr.io/pandaquq/robroz-runner:v0.1.0`.
- Manual platform screenshot workflow for Windows and macOS ([#153](https://github.com/PandaQuQ/robroz/pull/153), [#154](https://github.com/PandaQuQ/robroz/pull/154)).
- Client HTTPS via rustls, and persisted connection settings ([#152](https://github.com/PandaQuQ/robroz/pull/152)). The host address is stored in a `host` file next to `lang`. The token is stored in the OS keyring (macOS Keychain, Windows Credential Manager, Secret Service on Linux), falling back to a `token` file next to `lang` with mode `0600` on Unix. `ROBROZ_HOST` and `ROBROZ_TOKEN` override saved values for the current run and are never written back.
- In-app connection form with Host, Token, and Connect, on the offline pane and in Settings ([#159](https://github.com/PandaQuQ/robroz/pull/159)). A field set by the environment is read-only (`Set by ROBROZ_HOST` / `Set by ROBROZ_TOKEN`). When both are set by the environment, Connect is disabled. A non-local `http://` host shows `Unencrypted. Use HTTPS for remote hosts.` A 401 is titled `Token rejected` (「令牌被拒绝」), with the reason `The host rejected the token. Check ROBROZ_TOKEN.` An unreachable host is titled `Can't reach the host` (「连不上宿主」); the reason line is `Can't reach host.` (「连不上宿主。」) plus the detail.
- Chat feed rendered as bubbles ([#155](https://github.com/PandaQuQ/robroz/pull/155)). Your messages sit on the right; bro status, approval, choice, and confirm cards sit on the left. A delivery with `bot_name` is attached to that bro and follows the usual 2-minute grouping; a delivery without `bot_name` stands alone, with no avatar or name. On macOS the UI font is Hiragino Sans GB; on Windows it is Microsoft YaHei UI.
- Optional `taskId`, `botId`, and `botName` on `Event::Delivery` ([#158](https://github.com/PandaQuQ/robroz/pull/158)). The fields are omitted when absent, so older JSON serializes unchanged.

### Not yet merged

- TODO(theme): Appearance is not shipped (`feat/client-theme` is unmerged). Planned UI: a Settings row Appearance / 「外观」 under Language; segmented control System / 「跟随系统」, Light / 「浅色」, Dark / 「深色」; default System; follows the OS live; stored in `robroz/theme`; footer Dark toggle removed.
