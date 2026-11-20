#!/usr/bin/env python3
"""Generate the bundled Shijing lookup from the source edition in THIRD_PARTY_NOTICES.md.

Requires pypinyin 0.55.0 only when regenerating the checked-in asset.
"""

import argparse
import hashlib
import json
import re
from pathlib import Path

from pypinyin import lazy_pinyin, load_phrases_dict


SOURCE_SHA256 = "3c7ca54d59424542ada99adfcc85420287fbbd5c8a511e9baf705cb2b04051e7"
CLAUSE = re.compile(r"([^，。！？；：]+)([，。！？；：]?)")
END_SENTENCE = "。！？；"

# Readings in context where modern phrase dictionaries pick the wrong reading.
CLASSICAL_READINGS = {
    "将仲子": "qiang zhong zi",
    "薄言": "bo yan",
    "薄言还归": "bo yan huan gui",
    "予美亡此": "yu mei wu ci",
    "绿衣黄裳": "lv yi huang chang",
    "维参与昴": "wei shen yu mao",
    "契阔": "qi kuo",
    "说怿女美": "yue yi ru mei",
    "匪女之为美": "fei ru zhi wei mei",
    "洵美且都": "xun mei qie du",
    "王命召虎": "wang ming shao hu",
    "召公": "shao gong",
    "于嗟": "xu jie",
    "雨雪": "yu xue",
}


def phrases(poem):
    clauses = []
    sentences = []
    pending_text, pending_code = "", ""
    han_count = 0
    for stanza in poem["content"]:
        parts = CLAUSE.findall(stanza)
        if "".join(text + mark for text, mark in parts) != stanza:
            raise ValueError(f"Unparsed text in {poem['title']}: {stanza}")
        for text, mark in parts:
            reading = lazy_pinyin(text)
            if len(reading) != len(text) or any(not re.fullmatch("[a-z]+", s) for s in reading):
                raise ValueError(f"Missing pinyin in {poem['title']}: {text}: {reading}")
            code = "".join(reading)
            if len(text) >= 4:
                clauses.append((code, text))
            pending_code += code
            pending_text += text + mark
            han_count += len(text)
            if mark in END_SENTENCE and han_count >= 4:
                sentences.append((pending_code, pending_text))
                pending_text, pending_code, han_count = "", "", 0
    if han_count < 4 and pending_text:
        code, text = sentences.pop()
        pending_code = code + pending_code
        pending_text = text + pending_text
    if pending_text:
        sentences.append((pending_code, pending_text))
    if "".join(text for _, text in sentences) != "".join(poem["content"]):
        raise ValueError(f"Incomplete text in {poem['title']}")
    return clauses, sentences


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    source = args.source.read_bytes()
    if hashlib.sha256(source).hexdigest() != SOURCE_SHA256:
        parser.error("the source does not match the pinned Shijing edition")
    poems = json.loads(source)
    load_phrases_dict({phrase: [[s] for s in reading.split()]
                       for phrase, reading in CLASSICAL_READINGS.items()})
    all_clauses, all_sentences = [], []
    for poem in poems:
        clauses, sentences = phrases(poem)
        all_clauses.extend(clauses)
        all_sentences.extend(sentences)
    # Prefer a punctuation-free clause when the same code also matches a sentence.
    rows = dict.fromkeys(all_clauses + all_sentences)
    with args.output.open("w", encoding="utf-8", newline="\n") as output:
        output.write("# Full pinyin<TAB>original text. See THIRD_PARTY_NOTICES.md.\n")
        for code, text in sorted(rows, key=lambda row: row[0]):
            output.write(f"{code}\t{text}\n")
    print(f"Shijing: {len(poems)} poems, {len(rows)} unique pinyin entries")


if __name__ == "__main__":
    main()
