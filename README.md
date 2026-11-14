# Unexpected Keyboard [<img src="https://hosted.weblate.org/widget/unexpected-keyboard/svg-badge.svg" alt="État de la traduction" />](https://hosted.weblate.org/engage/unexpected-keyboard/)

## 中文说明

本项目基于 [Unexpected Keyboard](https://github.com/bqv/Unexpected-Keyboard) 修改。Unexpected Keyboard 是由 [Jules Aguillon（@Julow）](https://github.com/Julow) 发起的轻量 Android 键盘；本分支保留上游键盘功能，并加入中文输入和剪贴板同步。

### 本分支功能

- 拼音输入由 WowType Rust 引擎配合完整雾凇词库、补充字表和词级语言模型，提供整句候选、纠错、模糊音和本地学习；注音输入保留原有实现。
- 在文本密码框中自动切换到英文布局，并在底栏提供快速收起按钮。
- 内置中文词库；不同词库的数据来源说明见 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。
- 剪贴板默认持久保存最近 70 条文本；键盘打开时自动轮询远端更新，也可手动通过 SyncClipboard 服务器上传和下载文本、图片及文件，并提供预览和去重处理。密码框期间暂停自动轮询。
- 同步功能为可选项，需要在 UKB 应用设置中配置服务器。只有键盘打开后才会轮询，其他网络请求由用户手动触发。连接使用 HTTPS 和随应用提供的服务器证书。

### U / V 模式

在拼音布局开始新的输入串时使用以下前缀：

- **U 拆字查字**：输入部件的拼音，如 `umumu` 可选“林”、`uriyue` 可选“明”、`uhuohuohuo` 可选“焱”。支持未输完时的补全和隔音符；部件同音时可能有多个字，完整匹配优先。此模式不包含笔画编码。
- **V 计算与数字转换**：`v1+2*3` 得到 `7`，`v123.45` 提供“壹佰贰拾叁元肆角伍分”、中文数字和原数字。支持括号、负号、小数及 `+ - * / % ^`（乘除也可用 `× ÷`）。未完成或无效的算式保留为原式，不产生伪造结果。
- 点击候选或空格上屏首选，V 模式也可按 `=` 上屏首选。V 模式的数字用于算式，不用于选词；U 模式保留数字选词。退格和方向键编辑输入，Esc 取消，Enter 按现有行为上屏原始输入串。
- U/V 结果不写入普通拼音学习记录；普通拼音中的 `v`（如 `nv`）仍表示 ü。

### 中文输入状态与构建

拼音已完全切换到 [WowType](https://github.com/Gloridust/WowType)，不再回退到旧 Java 拼音引擎。当前随包数据约含 93 万条词条/读音记录，每次最多返回 1000 个候选，支持扩展区汉字。拼音纠错默认开启，模糊音默认关闭，可在设置中调整。覆盖安装后会按内容哈希更新词库与模型，并保留本地学习历史。

目前已通过 58 项 JVM 测试及模拟器上的候选、整句、纠错、部分选词、退格和数据更新检查。整体输入质量尚未完成与上游的系统对比；完整数据在 x86_64 模拟器中进程 PSS 约 458 MiB，内存、首次加载和分段组词学习仍需优化。具体结果见[移植与验证报告](doc/WOWTYPE_PORTING_REPORT.md)。

构建需要 Android SDK/NDK、Java、Python 3 和 Rust stable（rustup）。首次构建会下载固定版本的雾凇词库和语言模型，并校验模型 SHA-256；后续复用本地缓存。运行 `./gradlew assembleRelease` 构建发行版，签名配置及测试步骤见 [CONTRIBUTING.md](CONTRIBUTING.md#building-the-wowtype-pinyin-engine)。

### 上游署名与许可证

本项目基于 Jules Aguillon 及贡献者开发的 Unexpected Keyboard。保留了上游版权声明和 [`LICENSE`](LICENSE) 文件。本分支与上游采用相同的 GNU 通用公共许可证第 3 版（GPL-3.0），本分支的修改也按该许可证发布。内置词库、字体和其他第三方组件可能有各自的许可条件，请同时参阅对应的许可文件和说明。

## English

This is a fork of [Unexpected Keyboard](https://github.com/bqv/Unexpected-Keyboard), the lightweight Android keyboard started by [Jules Aguillon (@Julow)](https://github.com/Julow). It retains the upstream keyboard and adds Chinese input and clipboard sync.

### Features in this fork

- WowType Rust pinyin decoding with the full rime-ice lexicon, supplementary characters and word language model, offering sentence candidates, typo correction, fuzzy pinyin, and local learning. Zhuyin retains its existing implementation.
- Password text fields switch to an English layout automatically, and the bottom row has a quick-hide button.
- A bundled Chinese word dictionary. See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) for the separate dictionary sources.
- Clipboard history persistently keeps the latest 70 text entries by default. While the keyboard is open, it polls the configured SyncClipboard server for remote updates; users can also manually upload and download text, images, and files, with previews and duplicate-transfer handling. Polling pauses in password fields.
- Sync is optional and must be configured in the UKB app settings. Polling runs only while the keyboard is open; other network requests are user-triggered. The connection uses HTTPS and a certificate bundled for the configured server.

### U / V modes

Start a new composition on the Pinyin layout with `u` for component-pinyin
lookup (`umumu` → choose 林, `uriyue` → 明), or `v` for calculations
(`v1+2*3` → 7) and Chinese number/currency conversion (`v123.45`). U mode
supports completion and apostrophe separators, but not stroke codes. Homophone
components can produce multiple characters; exact readings rank before completions.

Tap a candidate or press Space to commit; `=` also commits the first V-mode
candidate. Digits enter numbers in V mode and select candidates in U mode.
Backspace and arrows edit, Esc cancels, and Enter commits the raw input.
Neither mode trains the pinyin dictionary. The `v` in ordinary pinyin such as
`nv` continues to represent ü. Invalid or unfinished expressions remain literal.

### Chinese input status and builds

Pinyin now uses [WowType](https://github.com/Gloridust/WowType) exclusively, with no fallback to the former Java decoder. Bundled data contains approximately 930,000 word/reading entries, with up to 1,000 candidates per query and support for supplementary Han characters. Typo correction defaults to on and fuzzy pinyin to off; both are configurable. App updates replace changed dictionary/model assets using content hashes while preserving local learning history.

Validation includes 58 JVM tests and emulator checks for candidates, sentences, typo correction, partial selection, backspace and data upgrades. Overall input quality has not yet been systematically compared with upstream. Process PSS with the full data was about 458 MiB on the x86_64 emulator; memory use, initial loading and learning phrases across partial selections still need work. See the [porting and validation report](doc/WOWTYPE_PORTING_REPORT.md) for details (Chinese).

Builds require the Android SDK/NDK, Java, Python 3 and stable Rust via rustup. The first build downloads pinned dictionary/model inputs and verifies the model SHA-256; later builds reuse the local cache. Run `./gradlew assembleRelease` for a release build. See [CONTRIBUTING.md](CONTRIBUTING.md#building-the-wowtype-pinyin-engine) for signing and test instructions.

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
