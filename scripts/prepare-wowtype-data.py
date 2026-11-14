#!/usr/bin/env python3
"""Reproduce WowType's default lexicon + word LM with pinned inputs."""
import hashlib, json, lzma, pathlib, shutil, subprocess, urllib.request
root = pathlib.Path(__file__).resolve().parent.parent
cache = root / 'build/wowtype-data'
out = root / 'build/generated-wowtype-assets'
cache.mkdir(parents=True, exist_ok=True)
out.mkdir(parents=True, exist_ok=True)
revision = '927e1cf1d3eeb9bf4be85e3f684dba4dd485b6f6'
lm_url = 'https://github.com/Gloridust/WowType/releases/download/lm-2026.11.09/words.lm.xz'
lm_sha = '42cb7c4a7cededa29b1a9ff105530085529fefff96535cc051341ebd51605dbe'
def digest(path):
    h=hashlib.sha256()
    with open(path,'rb') as f:
        for b in iter(lambda:f.read(1024*1024),b''):h.update(b)
    return h.hexdigest()
rime = cache / 'rime-ice'
if not (rime / '.git').exists():
    subprocess.run(['git','clone','--no-checkout','https://github.com/iDvel/rime-ice.git',str(rime)],check=True)
if subprocess.check_output(['git','-C',str(rime),'rev-parse','HEAD'],text=True).strip()!=revision:
    subprocess.run(['git','-C',str(rime),'fetch','origin',revision],check=True)
subprocess.run(['git','-C',str(rime),'checkout','--detach',revision],check=True)
with open(cache/'rime.tsv','w') as f:
    subprocess.run(['python3',str(root/'vendor/wowtype-data/rime2tsv.py'),'--extra-chars',str(rime/'cn_dicts/41448.dict.yaml'),str(rime/'rime_ice.dict.yaml')],stdout=f,check=True)
with open(out/'wowtype_lexicon.tsv','w') as f:
    subprocess.run(['python3',str(root/'vendor/wowtype-data/add_hotwords.py'),str(cache/'rime.tsv'),str(root/'vendor/wowtype-data/hotwords.tsv')],stdout=f,check=True)
lm=out/'wowtype_words.lm'
if not lm.exists() or digest(lm)!=lm_sha:
    packed=cache/'words.lm.xz'
    if not packed.exists():
        with urllib.request.urlopen(lm_url) as src, open(packed.with_suffix('.tmp'),'wb') as dst:shutil.copyfileobj(src,dst)
        packed.with_suffix('.tmp').replace(packed)
    with lzma.open(packed) as src,open(lm.with_suffix('.tmp'),'wb') as dst:shutil.copyfileobj(src,dst)
    if digest(lm.with_suffix('.tmp'))!=lm_sha:raise RuntimeError('Word LM SHA-256 mismatch')
    lm.with_suffix('.tmp').replace(lm)
# Component-pinyin table for U mode; preserve readings for candidate annotations.
rows=[]
body=False
for line in (rime/'radical_pinyin.dict.yaml').read_text().splitlines():
    if line.strip()=='...':body=True;continue
    if not body or not line.strip() or line.startswith('#'):continue
    fields=line.split('\t')
    if len(fields)<2:continue
    word,reading=fields[:2]
    code=reading.replace("'", "").replace(' ', '')
    if not code.isascii() or not code.isalpha():continue
    frequency=int(fields[2]) if len(fields)>2 and fields[2].isdigit() else 0
    rows.append((code,word,reading,frequency))
if not rows:raise RuntimeError('Empty component dictionary')
with open(out/'wowtype_components.tsv','w') as f:
    for row in sorted(rows):f.write('\t'.join(map(str,row))+'\n')
shutil.copyfile(rime/'LICENSE',out/'wowtype-data-LICENSE.txt')
(out/'wowtype-data-SOURCE.txt').write_text(f'''WowType default Chinese data pipeline, core commit 38eb269989a4c54c96c4b5749a13294750498b5e.
Lexicon: https://github.com/iDvel/rime-ice at {revision}, GPL-3.0.
Converted with WowType rime2tsv.py (including 41448 extra characters) and add_hotwords.py.
U-mode component readings: rime-ice radical_pinyin.dict.yaml (credits https://github.com/mirtlecn/rime_radical_pinyin).
Model: {lm_url}
SHA256: {lm_sha}
Upstream declares model CC BY-SA 4.0: https://creativecommons.org/licenses/by-sa/4.0/
Training data: LCCC (thu-coai, MIT), Chinese Wikipedia (CC BY-SA 4.0).
Attribution: WowType contributors, Tsinghua CoAI contributors, Wikipedia contributors.
Model distributed unmodified.
''')
files=['wowtype_lexicon.tsv','wowtype_words.lm','wowtype_components.tsv']
(out/'wowtype-data.json').write_text(json.dumps({f:digest(out/f) for f in files},sort_keys=True))
print('WowType data:',[(f,(out/f).stat().st_size) for f in files])
