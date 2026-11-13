# Unexpected Keyboard [<img src="https://hosted.weblate.org/widget/unexpected-keyboard/svg-badge.svg" alt="État de la traduction" />](https://hosted.weblate.org/engage/unexpected-keyboard/)

## 中文说明

本项目基于 [Unexpected Keyboard](https://github.com/bqv/Unexpected-Keyboard) 修改。Unexpected Keyboard 是由 [Jules Aguillon（@Julow）](https://github.com/Julow) 发起的轻量 Android 键盘；本分支保留上游键盘功能，并加入中文输入和剪贴板同步。

### 本分支功能

- 支持拼音和注音输入、中文词语候选、音节分段及候选选择优化。
- 在文本密码框中自动切换到英文布局，并在底栏提供快速收起按钮。
- 内置中文词库；词库来源和许可见 [`srcs/chinese/README.md`](srcs/chinese/README.md)。
- 剪贴板默认持久保存最近 70 条文本；键盘打开时自动轮询远端更新，也可手动通过 SyncClipboard 服务器上传和下载文本、图片及文件，并提供预览和去重处理。密码框期间暂停自动轮询。
- 同步功能为可选项，需要在 UKB 应用设置中配置服务器。只有键盘打开后才会轮询，其他网络请求由用户手动触发。连接使用 HTTPS 和随应用提供的服务器证书。

### 上游署名与许可证

本项目基于 Jules Aguillon 及贡献者开发的 Unexpected Keyboard。保留了上游版权声明和 [`LICENSE`](LICENSE) 文件。本分支与上游采用相同的 GNU 通用公共许可证第 3 版（GPL-3.0），本分支的修改也按该许可证发布。内置词库、字体和其他第三方组件可能有各自的许可条件，请同时参阅对应的许可文件和说明。

## English

This is a fork of [Unexpected Keyboard](https://github.com/bqv/Unexpected-Keyboard), the lightweight Android keyboard started by [Jules Aguillon (@Julow)](https://github.com/Julow). It retains the upstream keyboard and adds Chinese input and clipboard sync.

### Features in this fork

- Pinyin and Zhuyin input with Chinese word candidates, syllable segmentation, and candidate-selection improvements.
- Password text fields switch to an English layout automatically, and the bottom row has a quick-hide button.
- A bundled Chinese word dictionary. Its sources and licenses are documented in [`srcs/chinese/README.md`](srcs/chinese/README.md).
- Clipboard history persistently keeps the latest 70 text entries by default. While the keyboard is open, it polls the configured SyncClipboard server for remote updates; users can also manually upload and download text, images, and files, with previews and duplicate-transfer handling. Polling pauses in password fields.
- Sync is optional and must be configured in the UKB app settings. Polling runs only while the keyboard is open; other network requests are user-triggered. The connection uses HTTPS and a certificate bundled for the configured server.

### Upstream credit and license

This project is derived from Unexpected Keyboard by Jules Aguillon and its contributors. The upstream copyright notices and [`LICENSE`](LICENSE) are retained. This fork uses the same GNU General Public License, version 3 (GPL-3.0), as upstream, and distributes its modifications under that license. Bundled dictionaries, fonts, and other third-party components may have separate license terms; see their accompanying notices and documentation.

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
