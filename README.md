# Unexpected Keyboard [<img src="https://hosted.weblate.org/widget/unexpected-keyboard/svg-badge.svg" alt="État de la traduction" />](https://hosted.weblate.org/engage/unexpected-keyboard/)

## 中文说明

本项目基于 [Unexpected Keyboard](https://github.com/bqv/Unexpected-Keyboard) 修改。Unexpected Keyboard 是由 [Jules Aguillon（@Julow）](https://github.com/Julow) 发起的轻量 Android 键盘；本分支保留上游键盘功能，并加入中文输入和剪贴板同步。

### 下载与安装

从 [GitHub 发行版](https://github.com/limnop/ukb-chinese-input/releases/latest) 下载 APK，支持 Android 5.0 及以上系统。

| 安装包 | 用途 | Android 包名 |
| --- | --- | --- |
| `*-release.apk` | 日常使用，可覆盖安装同签名的旧版 | `juloo.keyboard2` |
| `*-debug.apk` | 测试与排查，可和正式版并装 | `juloo.keyboard2.debug` |

安装后打开应用，点击「启用键盘」和「选择键盘」，然后在「在此试用」文本框检查输入。需要使用中文软键盘时，在「设置 → 布局」中添加拼音或注音布局。若 Android 提示签名不同，先备份需要保留的数据，再考虑卸载冲突版本或使用可并装的调试版。

### 本分支功能

- 在「设置 → 样式 → 主题」按分类浏览配色。新增磷光终端、极简石墨和雾蓝亚克力：[键盘实拍对比](doc/keyboard-themes.png)。亚克力主题使用半透明色层。
- 拼音输入由 WowType v0.6.0 Rust 引擎配合完整雾凇词库、补充字表和词级语言模型，提供整句候选、纠错、模糊音和本地学习；注音输入保留原有实现。
- 拼音预装《诗经》305 篇正文，按分句和带标点的整句收录，每条至少四个汉字。输入全拼即可选中原句，例如 `guanguanjujiuzaihezhizhou` →「关关雎鸠，在河之洲。」。常用软件名支持原名拼写或中文全拼，例如 `chatgpt` → ChatGPT，`claudecode` → Claude Code，`tongyiqianwen` → 通义千问。
- 在拼音布局上，从 `x` 向左上滑输入 `「`，向右下滑输入 `」`；点按仍输入 `x`。
- 拼音和注音布局在分体键盘上也保留候选栏。退格优先编辑当前拼音，拼音清空后可继续删除已上屏的文字。
- 内置中文词库；不同词库的数据来源说明见 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。
- 剪贴板默认持久保存最近 70 条文本；软键盘打开时自动轮询远端更新。可手动通过 SyncClipboard 服务器上传和下载文本、图片及文件，并预览结果。重复下载时会复用仍保存在下载目录中的同一文件。密码框和实体键盘模式下暂停自动轮询。
- 同步功能为可选项，需要在 UKB 应用设置中配置服务器。只有键盘打开后才会轮询，其他网络请求由用户手动触发。连接使用 HTTPS 和系统信任的证书。自签名服务器可在「SyncClipboard 设置」中填写可信服务器证书的 SHA-256 指纹，例如使用 `openssl x509 -in server.pem -noout -fingerprint -sha256` 获取等号后的值。

### Emoji 面板

默认布局下，从底行 `Fn` 键向左下角的笑脸滑动打开面板。面板覆盖 Unicode Emoji 18.0 的 3,972 个标准条目。主栏显示常用、笑脸、手势、自然和食物，人物、旅行、活动、物品、符号和旗帜可从「更多」打开。

- 点按直接输入经典款。带角标的表情支持长按，随后点选肤色、性别、方向等变体。点「返回表情分类」可取消。
- 270 面旗帜各有独立的格子。选过的具体变体也可以在「常用」找到。
- 面板和独立的 Emoji 候选使用随包提供的 Noto Color Emoji 字体。普通文字和符号使用原有字体，上屏后的样式由接收应用决定。

在支持的应用中打开面板，默认选中该应用的原生表情分类，仍可通过主栏切换到通用 Emoji。内置 940 张离线静态预览。

| 应用 | 表情数 | 点按输入 |
| --- | ---: | --- |
| QQ | 254 | `U+0014` 加 Android 本地表情 ID 对应的码点 |
| 微信 | 110 | 半角方括号表情码，如 `[微笑]` |
| 抖音（含极速版） | 214 | 半角方括号表情码，如 `[酷拽]` |
| 哔哩哔哩（含全球版及 HD 版） | 362 | 半角方括号表情码，如 `[tv_doge]` |

接收应用决定表情码在当前输入场景中的显示方式。[表情资源来源](THIRD_PARTY_NOTICES.md)

### 文本密码键盘

文本密码框默认使用英文 QWERTY。顶行提供独立的 `1 2 3 4 5 6 7 8 9 0` 数字键，每个键只显示数字。底栏默认显示，提供空格、回车和右侧的收起按钮。在符号页点 `ABC` 可返回密码键盘。

### 实体键盘模式

连接 USB 或蓝牙键盘后，可在普通文本框中使用悬浮候选栏输入拼音。在 UKB 设置的「连接实体键盘时（含 scrcpy UHID）」中，默认选项为「实体键盘模式（悬浮拼音候选栏）」。候选栏悬浮在应用上方，应用输入框保持原位；当应用提供光标位置时，候选栏会避开底部的光标。原有的「隐藏所有」和「显示所有」可继续选择。

中文模式空闲时，右下角显示小「中」标志。有待确认的拼音时，标志让位给候选栏。英文模式下两者都隐藏，单按 Shift 可切回中文。

| 按键 | 功能 |
| --- | --- |
| 单按左 Shift 或右 Shift | 切换拼音和英文 |
| 空闲时点击右下角的「中」 | 从拼音切到英文 |
| 空格、点击候选或按下候选前的数字 | 上屏当前页首选或对应候选 |
| ↑ / ↓ 或 Page Up / Page Down | 候选翻页 |
| ← / →、Backspace、Delete | 编辑输入中的拼音 |
| Esc / Enter | 取消当前拼音 / 上屏原始拼音 |

中文模式自动转换逗号、句号等常用标点，U 拆字和 V 计算也可直接从实体键盘输入。密码、数字等非普通文本字段直接接收按键。

#### 通过 scrcpy 使用电脑键盘

在 UKB 中选用「实体键盘模式（悬浮拼音候选栏）」，然后用 UHID 模式启动 scrcpy（USB 和无线连接均可）。

```sh
scrcpy --keyboard=uhid
```

首次连接时，在 Android 的「实体键盘」设置中为 scrcpy 键盘选择与电脑匹配的布局。打开手机上的文本框并在 scrcpy 窗口内打字，即可使用上面的悬浮候选栏和快捷键。UHID 会让 Android 将电脑键盘识别为实体设备。参见 [scrcpy 键盘文档](https://github.com/Genymobile/scrcpy/blob/master/doc/keyboard.md#uhid)。

### U / V 模式

在拼音布局开始新的输入串时使用以下前缀：

- **U 拆字查字**：输入部件的拼音，如 `umumu` 可选“林”、`uriyue` 可选“明”、`uhuohuohuo` 可选“焱”。支持未输完时的补全和隔音符；部件同音时可能有多个字，完整匹配优先。此模式不包含笔画编码。
- **V 计算与数字转换**：`v1+2*3` 得到 `7`，`v123.45` 提供“壹佰贰拾叁元肆角伍分”、中文数字和原数字。支持括号、负号、小数及 `+ - * / % ^`（乘除也可用 `× ÷`）。未完成或无效的算式保留为原式，不产生伪造结果。
- 点击候选或空格上屏首选，V 模式也可按 `=` 上屏首选。V 模式的数字用于算式，不用于选词；U 模式保留数字选词。退格和方向键编辑输入，Esc 取消，Enter 按现有行为上屏原始输入串。
- U/V 结果不写入普通拼音学习记录；普通拼音中的 `v`（如 `nv`）仍表示 ü。

### 中文输入状态与构建

拼音已完全切换到 [WowType](https://github.com/Gloridust/WowType)，不再回退到旧 Java 拼音引擎。当前随包数据约含 93 万条词条/读音记录，每次最多返回 1000 个候选，支持扩展区汉字。拼音纠错默认开启，模糊音默认关闭，可在设置中调整。启动时会校验词库与模型，发现数据损坏时从 APK 恢复。覆盖安装和修复数据均保留本地学习历史。

2.3.2 的 Debug 和 Release 各通过 64 项 JVM 测试。模拟器验证了通用 Emoji 字形和变体、四个平台的原生表情分类和短码提交，以及文本密码框的数字行、底栏和符号页返回。原生表情短码使用模拟输入框验证。拼音的候选、整句、纠错、部分选词、退格、数据更新及精确匹配的验证结果见[移植与验证报告](doc/WOWTYPE_PORTING_REPORT.md)。载入完整拼音数据后，进程在 x86_64 模拟器中的 PSS 约为 458 MiB。整体输入质量仍待与上游做系统对比，内存、首次加载和分段组词学习仍有优化空间。

构建需要 Android SDK/NDK、Java、Python 3 和 Rust stable（rustup）。首次构建会下载固定版本的雾凇词库和语言模型，并校验模型 SHA-256；后续复用本地缓存。运行 `./gradlew assembleRelease` 构建发行版，签名配置及测试步骤见 [CONTRIBUTING.md](CONTRIBUTING.md#building-the-wowtype-pinyin-engine)。

发布前在 [build.gradle.kts](build.gradle.kts) 同步更新 `versionName` 和 `versionCode`，其中 `versionCode` 必须大于此前发出的安装包。新功能提高次版本号，修复提高修订号。正式版和调试版从同一提交构建，核对两包版本与签名后，用 `v<versionName>` 标签发布并上传两份 APK。详见[发布步骤](CONTRIBUTING.md#releasing-this-fork)。

### 上游署名与许可证

本项目基于 Jules Aguillon 及贡献者开发的 Unexpected Keyboard。保留了上游版权声明和 [`LICENSE`](LICENSE) 文件。本分支与上游采用相同的 GNU 通用公共许可证第 3 版（GPL-3.0），本分支的修改也按该许可证发布。内置词库、字体和其他第三方组件可能有各自的许可条件，请同时参阅对应的许可文件和说明。

## English

This is a fork of [Unexpected Keyboard](https://github.com/bqv/Unexpected-Keyboard), the lightweight Android keyboard started by [Jules Aguillon (@Julow)](https://github.com/Julow). It retains the upstream keyboard and adds Chinese input and clipboard sync.

### Download and install

Get the release or debug APK from [GitHub Releases](https://github.com/limnop/ukb-chinese-input/releases/latest). The keyboard supports Android 5.0 and later. Use the release APK for daily typing. The debug APK has a separate Android application ID and can be installed alongside it. Open the app to enable and select the keyboard, then add a Pinyin or Zhuyin layout in settings.

### Features in this fork

- Browse themes by category under Settings → Style → Theme. The three added themes are Phosphor Terminal, Graphite Minimal, and Frosted Acrylic. The acrylic theme uses translucent layers. [See the theme screenshots](doc/keyboard-themes.png).
- WowType v0.6.0 Rust pinyin decoding with the full rime-ice lexicon, supplementary characters and word language model, offering sentence candidates, typo correction, fuzzy pinyin, and local learning. Zhuyin retains its existing implementation.
- Pinyin includes the complete text of the 305 Shijing poems in entries of at least four Han characters. Type the full pinyin to select a clause or punctuated sentence. Bundled software names also match their full spelling or Chinese pinyin, such as `chatgpt` for ChatGPT and `claudecode` for Claude Code.
- On the pinyin layout, swipe up-left from `x` for `「` and down-right for `」`. Tap the key for `x`.
- Split layouts keep the candidate bar for pinyin and Zhuyin. Backspace edits the pinyin composition before deleting committed text.
- A bundled Chinese word dictionary. See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) for the separate dictionary sources.
- Clipboard history persistently keeps the latest 70 text entries by default. The on-screen keyboard polls the configured SyncClipboard server for remote updates while open. You can manually upload and download text, images, and files and preview the results. Repeated downloads reuse the same file while it remains in the downloads folder. Polling pauses in password fields and physical keyboard mode.
- Sync is optional and must be configured in the UKB app settings. Polling runs only while the keyboard is open; other network requests are user-triggered. Connections use HTTPS and the system trust store. For a self-signed server, enter the SHA-256 fingerprint from a trusted copy of the server certificate in SyncClipboard settings.

### Emoji panel

On the default layout, swipe down-left from `Fn` toward the smiley face. The panel includes all 3,972 standard Unicode Emoji 18.0 entries. The main bar shows frequently used emoji, smileys, gestures, nature and food. Open **More** for people, travel, activities, objects, symbols and flags. Tap to enter the default form. Hold a marked emoji to choose a skin tone, gender, direction, or other available variant. The panel and standalone emoji candidates use the bundled Noto Color Emoji font.

In supported apps, the panel opens to the app's native emoji category. The main bar still lets you switch to standard Unicode emoji. The keyboard bundles 940 static previews for offline use.

| App | Emoji | Inserted text |
| --- | ---: | --- |
| QQ | 254 | `U+0014` followed by the code point for the Android local face ID |
| WeChat | 110 | An ASCII-bracketed code such as `[微笑]` |
| Douyin (including Lite) | 214 | An ASCII-bracketed code such as `[酷拽]` |
| Bilibili (including the global and HD versions) | 362 | An ASCII-bracketed code such as `[tv_doge]` |

The receiving app and input field determine how these codes render. [Emoji asset sources](THIRD_PARTY_NOTICES.md)

### Text password keyboard

Text password fields open with English QWERTY. The top row contains ten dedicated digit keys from 1 through 0, with no secondary symbols. The bottom row is visible by default and includes Space, Enter and a hide button on the right. Tap `ABC` on the symbol layout to return to the password keyboard.

### Physical keyboard mode

USB and Bluetooth keyboards can compose pinyin in text fields using a floating candidate bar. In UKB settings, **Typing → Physical keyboard behavior (including scrcpy UHID)** defaults to **Physical keyboard mode (floating pinyin candidates)**. **Hide everything** and **Show everything** are also available.

The bar floats over the editor without resizing it. When the editor reports a caret position, the bar moves above a caret it would otherwise cover. In Chinese mode, the bottom-right `中` badge appears while idle and expands to a candidate bar while composing. English mode hides the badge and bar.

| Key or action | Behavior |
| --- | --- |
| Tap either Shift key by itself | Switch between pinyin and English |
| Tap the bottom-right `中` badge while idle | Switch to English |
| Space, tap a candidate, or press its displayed number | Commit the first or selected candidate on the current page |
| Up / Down or Page Up / Page Down | Page through candidates |
| Left / Right, Backspace, Delete | Edit the composition |
| Esc / Enter | Cancel the composition / commit the raw pinyin |

Chinese mode converts common punctuation and supports U component lookup and V calculations. Password and numeric fields receive keys directly.

#### Using scrcpy

Start scrcpy with `scrcpy --keyboard=uhid` over USB or wirelessly. On first connection, choose a layout for the scrcpy keyboard in Android’s physical keyboard settings that matches your computer’s layout. Focus a text field and type in the scrcpy window. UHID presents the computer keyboard to Android as a physical device. [scrcpy keyboard guide](https://github.com/Genymobile/scrcpy/blob/master/doc/keyboard.md#uhid).

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

Pinyin now uses [WowType](https://github.com/Gloridust/WowType) exclusively, with no fallback to the former Java decoder. Bundled data contains approximately 930,000 word/reading entries, with up to 1,000 candidates per query and support for supplementary Han characters. Typo correction defaults to on and fuzzy pinyin to off; both are configurable. Startup verifies the extracted dictionary and model, restores damaged files from the APK, and preserves local learning history.

The 2.3.2 Debug and Release variants each pass 64 JVM tests. Emulator checks cover Unicode emoji rendering and variants, native emoji categories and code insertion in test editors for all four apps, and the digit row, bottom row and return from the symbol layout in text password fields. The [porting and validation report](doc/WOWTYPE_PORTING_REPORT.md) covers pinyin candidates, sentences, typo correction, partial selection, backspace, data upgrades and exact matches. With the full pinyin data loaded, process PSS was about 458 MiB on the x86_64 emulator. A systematic comparison with upstream is pending. Memory use, initial loading and learning phrases across partial selections still need work.

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
