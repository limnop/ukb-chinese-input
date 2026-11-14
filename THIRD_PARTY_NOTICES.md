# Third-party notices

## WowType Rust core

`vendor/wowtype-core/` is derived from [Gloridust/WowType](https://github.com/Gloridust/WowType),
commit `38eb269989a4c54c96c4b5749a13294750498b5e`. The upstream `core/Cargo.toml`
declares `MIT OR Apache-2.0`. The license texts are included in
`vendor/wowtype-core/licenses/`.

The core embeds emoji keywords derived from Unicode CLDR. Unicode's license is
available at <https://www.unicode.org/license.txt>.

## Chinese dictionary

The generated WowType lexicon reuses the project's existing
`srcs/chinese/data/zh_pinyin.dat` (formerly `assets/zh_pinyin.dat`).
`srcs/chinese/build_pinyin_words.py` describes its RIME dictionary input pipeline
(rime-ice, unexpect-max luna_pinyin, and wanxiang). The exact source revisions
and per-source license inventory were not recorded with that asset; this port
does not establish new provenance or licensing for it.

The separate downloadable dictionary described in `srcs/chinese/README.md`
is derived from Jieba and Unicode Unihan. Its provenance must not be confused
with the bundled pinyin asset. The WowType converter filters the existing asset
by frequency and converts it to TSV; no additional vocabulary was downloaded.
