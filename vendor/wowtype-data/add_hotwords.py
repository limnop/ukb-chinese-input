#!/usr/bin/env python3
"""把热词表合进词库。

    python3 tools/add_hotwords.py lexicon.tsv data/hotwords.tsv [--freq 30000] > lexicon.new.tsv

词库里已有的词：每个读音的词频提到不低于 --freq（已经更高的不动）；没有的词按热词表的拼音加在末尾。
"""

import argparse
import sys


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("lexicon")
    ap.add_argument("hotwords")
    ap.add_argument("--freq", type=int, default=30000)
    args = ap.parse_args()

    hot = {}
    for line in open(args.hotwords, encoding="utf-8"):
        line = line.rstrip("\n")
        if not line or line.startswith("#"):
            continue
        pinyin, word = line.split("\t")[:2]
        hot.setdefault(word, pinyin)

    seen = set()
    raised = 0
    out = sys.stdout
    for line in open(args.lexicon, encoding="utf-8"):
        cols = line.rstrip("\n").split("\t")
        if len(cols) >= 3 and not line.startswith("#") and cols[1] in hot:
            seen.add(cols[1])
            try:
                freq = float(cols[2])
            except ValueError:
                freq = 0.0
            if freq < args.freq:
                cols[2] = str(args.freq)
                raised += 1
            out.write("\t".join(cols) + "\n")
        else:
            out.write(line)
    added = 0
    for word, pinyin in hot.items():
        if word not in seen:
            out.write(f"{pinyin}\t{word}\t{args.freq}\n")
            added += 1
    print(f"热词：提高词频 {raised} 条，新增 {added} 个", file=sys.stderr)


if __name__ == "__main__":
    main()
