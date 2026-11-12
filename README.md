# Unexpected Keyboard [<img src="https://hosted.weblate.org/widget/unexpected-keyboard/svg-badge.svg" alt="État de la traduction" />](https://hosted.weblate.org/engage/unexpected-keyboard/)

## Chinese input fork

This is a personal fork of [Unexpected Keyboard](https://github.com/bqv/Unexpected-Keyboard), the lightweight Android keyboard originally created by [Jules Aguillon (@Julow)](https://github.com/Julow). It keeps the upstream keyboard and adds Chinese input and clipboard sync features.

### Changes in this fork

- Pinyin and Zhuyin input with Chinese word candidates, syllable segmentation, and candidate selection improvements.
- A bundled Chinese word dictionary generated from the sources and licenses documented in [`srcs/chinese/README.md`](srcs/chinese/README.md).
- Clipboard history that persists the most recent 70 text entries, plus manual upload and download through a configured SyncClipboard server. Sync is optional and requires server settings in the UKB app.
- Image and file downloads from SyncClipboard with previews and duplicate-transfer protection.

This fork makes network requests only when a user invokes SyncClipboard. The SyncClipboard connection uses HTTPS and a bundled certificate for the configured server.

### Upstream attribution and license

This project is derived from Unexpected Keyboard by Jules Aguillon and its contributors. The upstream copyright notices and [`LICENSE`](LICENSE) are retained. This fork uses the same GNU General Public License, version 3 (GPL-3.0), as upstream; its modifications are distributed under that license as well. Individual bundled dictionaries, fonts, and other third-party components may have their own license terms; see their accompanying notices and documentation.

---

⚠️ Google wants to kill the open-source Android community.

See [keepandroidopen.org](https://keepandroidopen.org/) and the [F-Droid blog](https://f-droid.org/en/2025/09/29/google-developer-registration-decree.html).

---

[<img src="https://fdroid.gitlab.io/artwork/badge/get-it-on.png"
     alt="Get it on F-Droid"
     height="80">](https://f-droid.org/packages/juloo.keyboard2/)
[<img src="https://play.google.com/intl/en_us/badges/images/generic/en-play-badge.png"
     alt="Get it on Google Play"
     height="80">](https://play.google.com/store/apps/details?id=juloo.keyboard2)

Lightweight and privacy-conscious virtual keyboard for Android.

https://github.com/Julow/Unexpected-Keyboard/assets/2310568/28f8f6fe-ac13-46f3-8c5e-d62443e16d0d

The main feature is that you can type more characters by swiping the keys towards the corners.

This application was originally designed for programmers using Termux.
Now perfect for everyday use.

This application contains no ads and is open source.

Usage: to apply the symbols located in the corners of each key, slide your finger in the direction of the symbols. For example, the Settings are opened by sliding in the left down corner.

| <img src="/fastlane/metadata/android/en-US/images/phoneScreenshots/1.png" alt="Screenshot-1" /> | <img src="/fastlane/metadata/android/en-US/images/phoneScreenshots/2.png" alt="Screenshot-2"/> | <img src="/fastlane/metadata/android/en-US/images/phoneScreenshots/3.png" alt="Screenshot-3"/> |
| --- | --- | --- |
| <img src="/fastlane/metadata/android/en-US/images/phoneScreenshots/4.png" alt="Screenshot-4" /> | <img src="/fastlane/metadata/android/en-US/images/phoneScreenshots/5.png" alt="Screenshot-5" /> | <img src="/fastlane/metadata/android/en-US/images/phoneScreenshots/6.png" alt="Screenshot-6" /> |

## Help translate the application

Improve the application translations [using Weblate](https://hosted.weblate.org/engage/unexpected-keyboard/).

[<img src="https://hosted.weblate.org/widget/unexpected-keyboard/multi-auto.svg" alt="État de la traduction" />](https://hosted.weblate.org/engage/unexpected-keyboard/)

## Contributing

For instructions on building the application, see
[Contributing](CONTRIBUTING.md).

## Acknowledgement

The [NLnet foundation](https://nlnet.nl/) funded the work on the spell checking feature.
