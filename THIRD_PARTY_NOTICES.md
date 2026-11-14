# Third-party notices

## WowType Rust core

`vendor/wowtype-core/` is derived from [Gloridust/WowType](https://github.com/Gloridust/WowType),
commit `38eb269989a4c54c96c4b5749a13294750498b5e`. The upstream `core/Cargo.toml`
declares `MIT OR Apache-2.0`. The license texts are included in
`vendor/wowtype-core/licenses/`.

The core embeds emoji keywords derived from Unicode CLDR. Unicode's license is
available at <https://www.unicode.org/license.txt>.

## Bundled Chinese dictionary and language model

The default WowType lexicon is built from
[rime-ice](https://github.com/iDvel/rime-ice) revision
`927e1cf1d3eeb9bf4be85e3f684dba4dd485b6f6`, under GPL-3.0. It includes the
41448 supplementary character table and WowType's hotword additions.
Converters and hotwords come from the WowType core snapshot named above.
The rime-ice license and exact data attribution are included in APK assets
as `wowtype-data-LICENSE.txt` and `wowtype-data-SOURCE.txt`.

The unmodified word language model is from WowType release `lm-2026.11.09`,
licensed upstream under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/).
Attribution: WowType contributors; Tsinghua CoAI contributors (LCCC, MIT);
Wikipedia contributors (Chinese Wikipedia, CC BY-SA 4.0).
Its SHA-256 is `42cb7c4a7cededa29b1a9ff105530085529fefff96535cc051341ebd51605dbe`.

## Historical dictionary sources

`srcs/chinese/data/zh_pinyin.dat` is retained as historical build input but is
no longer used for the default APK. Its builder documents merged RIME inputs;
exact source revisions and a per-source license inventory were not recorded
with that asset. The separate dictionary described in `srcs/chinese/README.md`
uses Jieba and Unicode Unihan; see that document for its notices.
