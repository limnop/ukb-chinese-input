#!/usr/bin/env python3
"""Build a smaller WowType TSV lexicon from the app's pinyin word source.

Input rows are `concatenated-pinyin<TAB>frequency<TAB>word`. The upstream
engine accepts concatenated pinyin and performs its own syllable parsing.
"""
import os
import re
import sys

if len(sys.argv) != 4:
    raise SystemExit("usage: make-wowtype-lexicon.py INPUT SYLLABLE_RS OUTPUT.tsv")

source, syllable_rs, output = sys.argv[1:]
minimum_frequency = int(os.environ.get("WOWTYPE_MIN_FREQUENCY", "300"))
source_text = open(syllable_rs, encoding="utf-8").read()
match = re.search(r"pub const SYLLABLES: &\[&str\] = &\[(.*?)\];", source_text, re.S)
if not match:
    raise SystemExit("could not read the vendored WowType syllable table")
syllables = set(re.findall(r'"([a-z]+)"', match.group(1)))


def segment(code, count):
    """Segment a concatenated code into exactly one syllable per Han char."""
    memo = {}

    def visit(pos, left):
        key = (pos, left)
        if key in memo:
            return memo[key]
        if left == 0:
            return [] if pos == len(code) else None
        # Longest-first matches WowType's tie break, while character count
        # disambiguates codes such as xian for 先 versus 西安.
        for end in range(min(len(code), pos + 6), pos, -1):
            part = code[pos:end]
            if part not in syllables:
                continue
            rest = visit(end, left - 1)
            if rest is not None:
                memo[key] = [part] + rest
                return memo[key]
        memo[key] = None
        return None

    return visit(0, count)


def pinyin_for(code, word):
    count = len(word)
    parts = segment(code, count)
    if parts is None and code.endswith("r") and len(code) > 1:
        parts = segment(code[:-1], count - 1)
        if parts:
            parts[-1] += " r"
    return " ".join(parts) if parts else None


kept = 0
skipped_non_bmp = 0
skipped_unsegmentable = 0
with open(source, "rt", encoding="utf-8") as src, open(output, "wt", encoding="utf-8") as dst:
    for line_no, line in enumerate(src, 1):
        fields = line.rstrip("\n").split("\t")
        if len(fields) != 3:
            raise SystemExit(f"line {line_no}: expected 3 tab-separated fields")
        code, frequency, word = fields
        if int(frequency) < minimum_frequency:
            continue
        if any(ord(char) > 0xFFFF for char in word):
            skipped_non_bmp += 1
            continue
        pinyin = pinyin_for(code, word)
        if pinyin is None:
            skipped_unsegmentable += 1
            continue
        dst.write(f"{pinyin}\t{word}\t{frequency}\n")
        kept += 1
print(f"WowType lexicon: {kept} entries (frequency >= {minimum_frequency}); "
      f"skipped {skipped_unsegmentable} unsegmentable and {skipped_non_bmp} non-BMP rows")
