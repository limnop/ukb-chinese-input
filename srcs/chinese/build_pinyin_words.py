#!/usr/bin/env python3
"""Build assets/zh_pinyin.dat from RIME .dict.yaml word lists.

Several sources are merged: rime-ice's cn_dicts, the "unexpect-max"
luna_pinyin dictionary and the wanxiang dictionaries. Source lines are
`text<TAB>code<TAB>weight`, where code is a sequence of pinyin syllables
separated by spaces. Some sources write tone marks on the syllables
(`a\u0300 ba\u0301`), some write plain letters (`a ba`), and some use `v` for
u-umlaut. All of them are reduced to a bare lowercase code, because that is
what the user actually types ("ni hao" -> "nihao").

Weights are not comparable between sources: rime-ice uses hand-tuned figures
in the tens of thousands while luna_pinyin uses raw frequencies in the
millions. Each weight is compressed to `100 * log10(1 + weight)`, which puts
all sources on one scale and keeps the order inside a source. rime-ice is
given a small bonus because it is curated by hand. Very low weights are
dropped: they are mostly wrong readings of polyphonic characters, and they
would otherwise bury the words people actually type.

Output lines are `code<TAB>score<TAB>text`, sorted by code and then by
descending score. That order lets ChineseWordDictionary binary-search the line
offsets and read a whole prefix range without building a hash table. At most
[MAX_PER_CODE] words are kept for one code; a row of candidates is never
longer than a few dozen anyway.

Usage:
    python3 build_pinyin_words.py <output file> <source file or dir>...
"""

import glob
import math
import os
import sys
import unicodedata

MAX_WORD_CHARS = 12
MIN_SCORE = 120
MAX_PER_CODE = 400

# u-umlaut is written `v` in the pinyin the keyboard produces.
UMLAUT = str.maketrans({
    "\u00fc": "v", "\u01d6": "v", "\u01d8": "v", "\u01da": "v",
    "\u01dc": "v", "\u00dc": "v",
})


def norm_code(raw):
    """Strip spaces, tone marks and diacritics from a RIME pinyin code."""
    code = raw.strip().replace(" ", "").translate(UMLAUT)
    code = "".join(c for c in unicodedata.normalize("NFD", code)
                   if not unicodedata.combining(c))
    return code


def is_word(text):
    if not text or len(text) > MAX_WORD_CHARS:
        return False
    for ch in text:
        o = ord(ch)
        if not (0x3400 <= o <= 0x9fff or 0xf900 <= o <= 0xfaff):
            return False
    return True


def is_code(code):
    return 0 < len(code) <= 40 and code.isascii() and code.isalpha() \
        and code.islower()


def source_factor(path):
    return 1.2 if "rime-ice" in path else 1.0


def expand(inputs):
    paths = []
    for src in inputs:
        if os.path.isdir(src):
            paths += sorted(glob.glob(os.path.join(src, "*.dict.yaml")))
        else:
            paths.append(src)
    return paths


def main():
    out_path, inputs = sys.argv[1], sys.argv[2:]
    paths = expand(inputs)
    best = {}
    rows = 0
    for path in paths:
        factor = source_factor(path)
        found = 0
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                if not line or line[0] == "#":
                    continue
                parts = line.rstrip("\n").split("\t")
                if len(parts) < 2:
                    continue
                text = parts[0].strip()
                code = norm_code(parts[1])
                if not is_word(text) or not is_code(code):
                    continue
                # A two-column file is text + weight, not text + code.
                if len(parts) == 2 and parts[1].strip().isdigit():
                    continue
                try:
                    weight = int(parts[2]) if len(parts) > 2 else 0
                except ValueError:
                    weight = 0
                score = int(factor * 100 * math.log10(1 + max(0, weight)))
                if score < MIN_SCORE:
                    continue
                key = (code, text)
                if score > best.get(key, -1):
                    best[key] = score
                rows += 1
                found += 1
        print("%-32s %9d rows" % (os.path.basename(path), found))
    by_code = {}
    for (code, text), score in best.items():
        by_code.setdefault(code, []).append((score, text))
    entries = 0
    with open(out_path, "w", encoding="utf-8", newline="\n") as f:
        for code in sorted(by_code):
            items = sorted(by_code[code], key=lambda e: (-e[0], e[1]))
            if len(items) > MAX_PER_CODE:
                items = items[:MAX_PER_CODE]
            for score, text in items:
                f.write("%s\t%d\t%s\n" % (code, score, text))
                entries += 1
    print("source rows: %d" % rows)
    print("entries:     %d" % entries)
    print("codes:       %d" % len(by_code))
    print("bytes:       %d (%.1f MB)" %
          (os.path.getsize(out_path), os.path.getsize(out_path) / 1048576.0))


if __name__ == "__main__":
    main()
