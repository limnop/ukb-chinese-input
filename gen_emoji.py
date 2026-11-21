"""Generate the emoji panel catalogue from the pinned Unicode keyboard data."""

import argparse
from collections import defaultdict
from pathlib import Path
import urllib.request

VERSION = '18.0'
EMOJI_TEST_URL = f'https://www.unicode.org/Public/{VERSION}.0/emoji/emoji-test.txt'
OUTPUT = Path(__file__).resolve().parent / 'res/raw/emojis.txt'

GROUPS = {
    'Smileys & Emotion': 'FACES',
    'People & Body': 'PEOPLE',
    'Animals & Nature': 'NATURE',
    'Food & Drink': 'FOOD',
    'Travel & Places': 'TRAVEL',
    'Activities': 'ACTIVITIES',
    'Objects': 'OBJECTS',
    'Symbols': 'SYMBOLS',
    'Flags': 'FLAGS',
}
GROUP_ORDER = ('FACES', 'GESTURES', 'PEOPLE', 'NATURE', 'FOOD', 'TRAVEL',
               'ACTIVITIES', 'OBJECTS', 'SYMBOLS', 'FLAGS')

SKIN_TONES = range(0x1F3FB, 0x1F400)
PERSON = 0x1F9D1
MAN, WOMAN = 0x1F468, 0x1F469
ZWJ = 0x200D
PRESENTATION = 0xFE0F


def read_entries(source):
    if f'# Version: {VERSION}' not in source.splitlines():
        raise ValueError(f'Expected Unicode Emoji {VERSION}')

    entries = {}
    for line in source.splitlines():
        if line.startswith('# group: '):
            group = line.removeprefix('# group: ')
        elif line.startswith('# subgroup: '):
            subgroup = line.removeprefix('# subgroup: ')
        elif line and not line.startswith('#'):
            codes, qualification = line.split('#', 1)[0].split(';')
            if qualification.strip() not in ('fully-qualified', 'component'):
                continue
            emoji = tuple(int(code, 16) for code in codes.split())
            if group == 'Component':
                category = 'GESTURES' if subgroup == 'skin-tone' else 'PEOPLE'
            elif group == 'People & Body' and subgroup.startswith(('hand', 'body-')):
                category = 'GESTURES'
            else:
                category = GROUPS[group]
            entries[emoji] = (category, subgroup)
    return entries


def family_key(base, subgroup, bases):
    # Unicode uses both an older single character and joined people/hands
    # for several emoji. Each couple pairing has its own set of skin tones.
    special = {
        (0x1FAF1, ZWJ, 0x1FAF2): (0x1F91D,),  # handshake
        (WOMAN, ZWJ, 0x1F91D, ZWJ, WOMAN): (0x1F46D,),
        (WOMAN, ZWJ, 0x1F91D, ZWJ, MAN): (0x1F46B,),
        (MAN, ZWJ, 0x1F91D, ZWJ, MAN): (0x1F46C,),
        (PERSON, ZWJ, 0x2764, ZWJ, 0x1F48B, ZWJ, PERSON): (0x1F48F,),
        (PERSON, ZWJ, 0x2764, ZWJ, PERSON): (0x1F491,),
    }
    if base in special:
        return special[base]
    if len(base) == 5 and base[0] == base[-1] and base[1::2] == (ZWJ, ZWJ):
        pair_activity = {0x1F430: 0x1F46F, 0x1FAEF: 0x1F93C}
        if base[2] in pair_activity:
            activity = (pair_activity[base[2]],)
            gender = {MAN: 0x2642, WOMAN: 0x2640}.get(base[0])
            return activity + (ZWJ, gender) if gender else activity

    if subgroup == 'family' and base not in bases:
        # Some mixed-tone couples use the reverse order of the classic pair.
        reverse = base[-1:] + base[1:-1] + base[:1]
        if reverse in bases:
            return reverse
    if subgroup in ('family', 'person-symbol') and len(base) > 1:
        if all(code in (PERSON, MAN, WOMAN, 0x1F9D2, 0x1F466, 0x1F467, ZWJ)
               for code in base):
            return (0x1F46A,)

    counterparts = {
        (0x1F466,): (0x1F9D2,),  # boy → child
        (0x1F467,): (0x1F9D2,),  # girl → child
        (0x1F474,): (0x1F9D3,),  # old man → older person
        (0x1F475,): (0x1F9D3,),  # old woman → older person
    }
    if base in counterparts:
        return counterparts[base]
    if subgroup.startswith('person') and subgroup != 'person-symbol':
        # Walking, running and mobility sequences can specify a direction.
        if base[-2:] == (ZWJ, 0x27A1) and base[:-2] in bases:
            base = base[:-2]
        # Occupations and hairstyles can use the same joiner sequence with
        # a woman, man or gender-neutral person.
        neutral = (PERSON,) + base[1:]
        if base[0] in (MAN, WOMAN) and neutral in bases:
            return neutral
        # Some activities express gender using a sign at the end instead.
        if base[-2:] in ((ZWJ, 0x2640), (ZWJ, 0x2642)):
            # Each of these has 26 variants per gender, including mixed tones.
            if base[:-2] in bases and base[0] not in (0x1F46F, 0x1F93C):
                return base[:-2]
    return base


def render(source):
    entries = read_entries(source)
    bases = {
        tuple(code for code in emoji if code != PRESENTATION): emoji
        for emoji in entries
    }
    groups = defaultdict(dict)
    for emoji, (category, subgroup) in entries.items():
        base = tuple(code for code in emoji if code != PRESENTATION)
        if category != 'FLAGS' and subgroup not in ('skin-tone', 'hair-style'):
            base = tuple(code for code in base if code not in SKIN_TONES)
            base = family_key(base, subgroup, bases)
        representative = bases[base]
        # A family can never cross a tab boundary.
        assert entries[representative][0] == category, emoji
        groups[category].setdefault(representative, []).append(emoji)

    lines = [f'# Unicode Emoji {VERSION} · https://www.unicode.org/license.txt']
    for category in GROUP_ORDER:
        lines.append(f'[{category}]')
        for representative, variants in groups[category].items():
            variants.remove(representative)
            variants.insert(0, representative)
            lines.append('\t'.join(''.join(map(chr, emoji)) for emoji in variants))
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path, nargs='?', help='local Unicode emoji-test.txt')
    args = parser.parse_args()
    if args.source:
        source = args.source.read_text(encoding='utf-8')
    else:
        with urllib.request.urlopen(EMOJI_TEST_URL, timeout=30) as response:
            source = response.read().decode('utf-8')
    OUTPUT.write_text(render(source), encoding='utf-8')


if __name__ == '__main__':
    main()
