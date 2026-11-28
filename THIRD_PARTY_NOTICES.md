# Third-party notices

## Emoji panel

The panel catalogue is derived from [Unicode Emoji 18.0 keyboard data](https://www.unicode.org/Public/18.0.0/emoji/emoji-test.txt).
The Unicode License v3 is bundled as `assets/Unicode-Emoji-LICENSE.txt`.

`assets/NotoColorEmoji.ttf` is the unmodified Noto Color Emoji 2D font (v2.057)
from [Google's Unicode 18.0 release](https://github.com/googlefonts/noto-emoji/releases/tag/v2026-09-24-unicode18_0).
The SIL Open Font License 1.1 is bundled as `assets/NotoColorEmoji-LICENSE.txt`.

## Native app emoji previews

The static WebP previews in `assets/native-emoji/` come from the following
sources. Each accompanying TSV preserves the app's text code or Android local QQ face ID.
Rights to the artwork and source asset data remain with the respective
platforms and other rights holders.

- QQ face IDs, names and images come from [QFace's QQNT catalog](https://github.com/koishijs/QFace/tree/f040ba23d94dd43f203ec2da75d6ee78a18b1dc4). The catalog used here includes visible numeric-ID faces of types 0 and 1. The input code uses `U+0014` followed by the Android local face ID. Tencent's Android configuration maps server IDs (`QSid`) to local IDs (`AQLid`). The Android 9.3.85 system-face code uses the server ID for faces without a local override. [Tencent face configuration](https://downv6.qq.com/qqface/res/Config/face_config_202403251616.zip) · [TIM text encoding analysis](https://blog.qsboy.com/tim-reverse/)
- WeChat uses the newer WeChat set from [emoji-chinese](https://github.com/YiJio/emoji-chinese/tree/79c2292b778cf93cb5850f70c3168bcbfe39aeeb), including its `_map/wxNew.json` mappings.
- Douyin uses visible, non-time-limited entries from the Android small-emoji catalog in [douyin-emoji](https://github.com/hnlyzxf/douyin-emoji/tree/e1e0395662634c48c4073a6d8fba6138121a53d2), resource revision `1782206497228`.
- Bilibili uses inline emoji from the 小黄脸, 小电视 and 热词系列一 [comment emoji packs](https://api.bilibili.com/x/emote/package?business=reply&ids=1,2,53) and their Bilibili CDN images.

## WowType Rust core

`vendor/wowtype-core/` is derived from [Gloridust/WowType](https://github.com/Gloridust/WowType),
v0.6.0, commit `bb2a04fd573f1e5f2444ee68f8f71b7ea429b3c5`. The upstream `core/Cargo.toml`
declares `MIT OR Apache-2.0`. The license texts are included in
`vendor/wowtype-core/licenses/`.

The core embeds emoji keywords derived from Unicode CLDR. Unicode's license is
available at <https://www.unicode.org/license.txt>.
The simplified-to-traditional conversion table is derived from OpenCC;
its license is included in `vendor/wowtype-core/data/OPENCC-LICENSE`.

## Bundled Chinese dictionary and language model

The default WowType lexicon is built from
[rime-ice](https://github.com/iDvel/rime-ice) revision
`927e1cf1d3eeb9bf4be85e3f684dba4dd485b6f6`, under GPL-3.0. It includes the
41448 supplementary character table and WowType's hotword additions.
U-mode component lookup uses `radical_pinyin.dict.yaml` from the same pinned
rime-ice tree; its header credits [rime_radical_pinyin](https://github.com/mirtlecn/rime_radical_pinyin).
The generated table preserves component readings and frequencies.
Converters and hotwords remain pinned to the data revision in `vendor/wowtype-data/UPSTREAM.md`.
The rime-ice license and exact data attribution are included in APK assets
as `wowtype-data-LICENSE.txt` and `wowtype-data-SOURCE.txt`.

The unmodified word language model is from WowType release `lm-2026.11.09`,
licensed upstream under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/).
Attribution: WowType contributors; Tsinghua CoAI contributors (LCCC, MIT);
Wikipedia contributors (Chinese Wikipedia, CC BY-SA 4.0).
Its SHA-256 is `42cb7c4a7cededa29b1a9ff105530085529fefff96535cc051341ebd51605dbe`.

## Bundled Shijing text

The Shijing entries are generated from the 305 poems in
[chinese-poetry's Shijing transcription](https://github.com/chinese-poetry/chinese-poetry/blob/b8594f81a89752241442f2ce267d6f66f96704ee/%E8%AF%97%E7%BB%8F/shijing.json)
at commit `b8594f81a89752241442f2ce267d6f66f96704ee`.
The ancient text is in the public domain. The source edition's MIT notice is
included in the APK as `assets/wowtype-shijing-LICENSE.txt`.
`scripts/make-shijing-phrases.py` generates the exact-match input codes with
[pypinyin](https://github.com/mozillazg/python-pinyin) 0.55.0 and contextual reading
overrides. The generated asset is checked in, so Android builds require no
additional download or Python package.

## Historical dictionary sources

`srcs/chinese/data/zh_pinyin.dat` is retained as historical build input but is
no longer used for the default APK. Its builder documents merged RIME inputs;
exact source revisions and a per-source license inventory were not recorded
with that asset. The separate dictionary described in `srcs/chinese/README.md`
uses Jieba and Unicode Unihan; see that document for its notices.
