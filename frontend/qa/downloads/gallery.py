#!/usr/bin/env python3
"""Assemble native-render captures into an offline comparison gallery. Requires Pillow.
Usage: python gallery.py /tmp/tau2-download-qa
The root contains tau1/, before/ (optional), after/. No UI is recreated by this script.
"""
import html, json, math, pathlib, sys
from PIL import Image, ImageDraw, ImageFont
root = pathlib.Path(sys.argv[1])
cases = json.loads((pathlib.Path(__file__).parent / 'cases.json').read_text())
try:
    font = ImageFont.truetype('/usr/share/fonts/TTF/DejaVuSans.ttf', 12)
except OSError:
    font = ImageFont.load_default()
profiles = ['desktop', 'sidebar', 'phone', 'phone-2x5']
parts = ['''<!doctype html><meta charset="utf-8"><title>Tau inline downloads: native render comparison</title>
<style>body{background:#090d12;color:#e5eaf0;font:14px system-ui;margin:24px;max-width:1900px}a{color:#67d4ff}h2{margin-top:50px}h3{margin-top:30px}.row{display:flex;gap:16px;align-items:flex-start}.cell{flex:1;min-width:0;background:#0e141b;padding:10px;border-radius:12px}img{max-width:100%;height:auto;display:block}summary{cursor:pointer;color:#67d4ff;padding:12px}small,p{color:#b7c2ce}nav a{margin-right:16px}</style>
<h1>Inline file downloads</h1><p>Original Tau1 Compose controls · previous Tau2 · revised Tau2, rendered natively from the same fixture data. No HTML mock UI.</p>
<p>Tau1: 3818579. Its original 68dp control is extracted unchanged except no-op callbacks, platform simulation and layout probes. Tau1 has no separate OS-saving state (shown at its final downloading frame), no View button (preview click), and no download tooltips. Image preview comparisons show the Tau1 <em>control</em>, not its full image viewer. The previous-Tau2 column is optional historical evidence.</p>
<p>32 lifecycle/content cases × desktop, sidebar, phone, 2.5× phone. Revised Tau2 also renders every applicable icon hovered, pressed, with its tooltip, and partially scrolled out. These headless Linux renders are not physical Windows/Android acceptance.</p><nav>''']
parts += [f'<a href="#{p}">{p}</a>' for p in profiles]
parts.append('</nav>')
for profile in profiles:
    parts.append(f'<h2 id="{profile}">{profile}</h2>')
    for case in cases:
        stem = f'{profile}-{case["id"]}'
        parts.append(f'<h3>{html.escape(case["label"])}</h3><div class="row">')
        for folder, label in [('tau1','Tau1 · original Compose'),('before','Tau2 · before'),('after','Tau2 · revised')]:
            image = root / folder / f'{stem}.png'
            parts.append(f'<div class="cell"><b>{label}</b>')
            if image.exists(): parts.append(f'<img loading="lazy" src="{folder}/{image.name}">')
            else: parts.append('<p>No historical capture</p>')
            parts.append('</div>')
        parts.append('</div><details><summary>All button interactions + scroll clipping</summary>')
        for folder in ['tau1','after']:
            for state in ['hover','pressed','tooltip','clipped']:
                shots = sorted((root/folder).glob(f'{stem}-{state}*.png'))
                if not shots: continue
                parts.append(f'<h4>{folder} · {state}</h4><div class="row">')
                for shot in shots: parts.append(f'<div class="cell"><img loading="lazy" src="{folder}/{shot.name}"></div>')
                parts.append('</div>')
        parts.append('</details>')
parts.append('<h2>Actual application surfaces</h2><div class="row">')
for shot in sorted((root/'after').glob('context-*.png')):
    parts.append(f'<div class="cell"><p>{shot.stem}</p><img loading="lazy" src="after/{shot.name}"></div>')
parts.append('</div>')
(root/'index.html').write_text('\n'.join(parts))
# Four dense sheets show every control state, including the image control strip.
for page in range(math.ceil(len(cases)/8)):
    batch=cases[page*8:(page+1)*8]
    canvas=Image.new('RGB',(960,48+len(batch)*148),(9,13,18)); draw=ImageDraw.Draw(canvas)
    for col,label in enumerate(['TAU1 · ORIGINAL COMPOSE','TAU2 · BEFORE','TAU2 · REVISED']):
        draw.text((col*320+12,14),label,fill='#67d4ff',font=font)
    for row,c in enumerate(batch):
        top=48+row*148
        draw.text((12,top+4),c['label'],fill='#e5eaf0',font=font)
        for col,folder in enumerate(['tau1','before','after']):
            p=root/folder/f'sidebar-{c["id"]}.png'
            if not p.exists(): continue
            im=Image.open(p).convert('RGB')
            if folder=='tau1': crop=(12,44,308,116)
            elif folder=='before':
                y=44+(240 if c.get('image') else 0)
                crop=(12,y,308,y+104)
            else:
                h=84+(224 if c.get('image') else 0)+(24 if c.get('caption') else 0)
                y=44+h-76
                crop=(12,y-4,308,y+72)
            canvas.paste(im.crop(crop),(col*320+12,top+30))
    canvas.save(root/f'all-controls-{page+1:02}.png')
# Image previews are shown separately so the control sheets remain legible.
images=[c for c in cases if c.get('image')]
canvas=Image.new('RGB',(360*3,420*math.ceil(len(images)/3)),(9,13,18))
for n,c in enumerate(images):
    canvas.paste(Image.open(root/'after'/f'phone-{c["id"]}.png').convert('RGB'),((n%3)*360,(n//3)*420))
canvas.save(root/'image-previews.png')
print(f'Wrote {root}/index.html and all state contact sheets')
