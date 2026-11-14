# WowType data pipeline

`rime2tsv.py`, `add_hotwords.py`, and `hotwords.tsv` are copied unchanged from
Gloridust/WowType commit `38eb269989a4c54c96c4b5749a13294750498b5e`.
See ../wowtype-core/licenses for WowType's MIT / Apache-2.0 license texts.

scripts/prepare-wowtype-data.py reproduces the default data pipeline from
WowType's scripts/make-data.sh, using pinned rime-ice revision
927e1cf1d3eeb9bf4be85e3f684dba4dd485b6f6 and the upstream checksum for the
lm-2026.11.09 word model. The generated APK assets include data attribution,
the rime-ice GPL-3.0 license, and a SHA-256 manifest. Binary model data is
cached under build/, not checked into git.
