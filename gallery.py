from pathlib import Path
import json, html, base64, os, zipfile
root=Path(__file__).resolve().parent
m=json.loads((root/'manifest.json').read_text())
builds=json.loads((root/'builds.json').read_text())
coverage=[
 ('Native chip buttons, labels, pressed/expanded state, keyboard behavior','01, 02, 03, 12'),
 ('Explicit React / Switch / Remove list actions','04, 05, 06, 17'),
 ('Phone and tablet touch inspection','07, 08, 09'),
 ('Standing reaction order, removal / re-add, optimistic pending order','10, 11, 19'),
 ('Last-chip focus fallback and exclusive list handoff','13, 14'),
 ('Dense wrapping, own-row tint, light / dark themes','15, 16, 18'),
]
css='''
*{box-sizing:border-box}body{margin:0;background:var(--background,#f6f7fa);color:var(--foreground,#17202c);font:14px/1.5 var(--font-sans,system-ui,sans-serif)}main{max-width:1500px;margin:auto;padding:28px 24px 45px}h1{font-size:30px;line-height:1.15;margin:0 0 12px;letter-spacing:-.03em}h2{font-size:19px;margin:0 0 4px}p{margin:8px 0 16px}.muted,summary{color:var(--muted-foreground,#5e6875)}.meta{display:flex;gap:9px;flex-wrap:wrap;margin:15px 0}.badge{border:1px solid var(--border,#dbe0e8);border-radius:20px;padding:3px 10px;background:var(--card,#fff);font-size:12px}code{font:12px var(--font-mono,monospace)}nav{display:flex;gap:6px;flex-wrap:wrap;position:sticky;top:0;background:var(--background,#f6f7fa);padding:12px 0;z-index:3}button{font:inherit;color:var(--foreground,#17202c);border:1px solid var(--border,#dbe0e8);background:var(--card,#fff);border-radius:8px;padding:6px 11px;cursor:pointer}button.active,button:hover{background:var(--accent,#e5edff);color:var(--accent-foreground,#173b82)}article{margin:16px 0 22px;border:1px solid var(--border,#dbe0e8);border-radius:14px;background:var(--card,#fff);padding:18px;scroll-margin-top:70px}article[hidden]{display:none}.pair{display:grid;grid-template-columns:1fr 1fr;gap:14px;margin:12px 0}.shot{margin:0;min-width:0}.label{display:flex;justify-content:space-between;align-items:center;font-size:12px;font-weight:650;margin:0 0 8px}.label b{padding:3px 8px;border-radius:5px;background:var(--muted,#edf0f5)}.shot img{display:block;width:100%;height:auto;border:1px solid var(--border,#dbe0e8);background:white;border-radius:7px}.shot.phone img{width:auto;max-width:100%;margin:0 auto}.shot.phone{background:var(--background,#f6f7fa);border-radius:8px;padding:10px}.shot.phone .label{max-width:390px;margin:0 auto 8px}.note{font-size:13px;color:var(--muted-foreground,#5e6875);margin:3px 0}a{color:var(--primary,#2456b5);text-underline-offset:2px}.facts{display:grid;grid-template-columns:1fr 1fr;gap:14px;margin-top:10px}.facts pre{overflow:auto;background:var(--muted,#edf0f5);padding:10px;border-radius:7px;font-size:11px;white-space:pre-wrap;overflow-wrap:anywhere}details{font-size:12px;margin-top:12px}summary{cursor:pointer}table{border-collapse:collapse;width:100%;font-size:12px}td,th{text-align:left;padding:7px 9px;border-bottom:1px solid var(--border,#dbe0e8)}.coverage{padding:18px;border:1px solid var(--border,#dbe0e8);border-radius:14px;background:var(--card,#fff);margin-top:26px}.kicker{font:600 11px var(--font-mono,monospace);text-transform:uppercase;letter-spacing:.08em;color:var(--muted-foreground,#5e6875);margin-bottom:8px}.counter{margin-left:auto;align-self:center;font-size:12px;color:var(--muted-foreground,#5e6875)}@media(max-width:650px){main{padding:18px 12px}.pair,.facts{grid-template-columns:1fr}h1{font-size:25px}article{padding:12px}nav{position:static}}@media print{nav,button{display:none}article{break-inside:avoid}.pair{grid-template-columns:1fr 1fr}main{padding:0}}
'''
def esc(s):return html.escape(str(s))
def state(d):
 return json.dumps({'chips':[{'text':c['text'],'tag':c['tag'],'aria-label':c['label'],'aria-pressed':c['pressed'],'aria-expanded':c['expanded'],'focused':c['focused']} for c in d['chips']], 'reactorLists':len(d['lists']),'focus':d['focus']['testid'] or d['focus']['tag'],'pointer':d['pointer'],'overflow':d['overflow']},ensure_ascii=False,indent=2)
def article(s,inline=False):
 a=[f'<article data-group="{esc(s["group"])}" id="{s["id"]}"><div class="kicker">{esc(s["id"].split("-")[0])} · {esc(s["group"])}</div><h2>{esc(s["title"])}</h2><p class="note">{esc(s["note"])}</p><div class="pair">']
 for v in ['before','after']:
  d=s['versions'][v];mobile=d['viewport']['width']<768
  img=('data:image/png;base64,'+base64.b64encode((root/d['image']).read_bytes()).decode()) if inline else d['image']
  link='' if inline else f'<a href="{d["full"]}" target="_blank">Full UI ↗</a>'
  a.append(f'<figure class="shot {"phone" if mobile else ""}"><figcaption class="label"><b>{v.title()} · {m["commits"][v][:8]}</b><span>{d["viewport"]["width"]}px · {d["theme"]}</span>{link}</figcaption><img src="{img}" alt="{v.title()}: {esc(s["title"])}" loading="lazy"></figure>')
 a.append('</div><details><summary>DOM / accessibility facts</summary><div class="facts">')
 for v in ['before','after']:a.append('<div><b>'+v.title()+'</b><pre>'+esc(state(s['versions'][v]))+'</pre></div>')
 a.append('</div></details></article>');return ''.join(a)
groups=['All']+list(dict.fromkeys(s['group'] for s in m['scenarios']))
head=f'''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>River reaction actions — before and after</title><style>{css}</style><main><div class="kicker">River · actual browser screenshots</div><h1>Reaction actions: before and after</h1><p class="muted">{len(m['scenarios'])} matched comparisons from the actual parent and finished builds. Images are unaltered browser screenshots; each also has a full-window capture.</p><div class="meta"><span class="badge">Before <code>fc92f19b</code></span><span class="badge">After <code>b5ea6bcb</code></span><span class="badge">Chromium {esc(m['browser'])}</span><span class="badge">Same fixture seed · fixed UTC clock · 1× pixels</span></div><nav>'''
head+=''.join(f'<button data-filter="{esc(g)}" class="{"active" if g=="All" else ""}">{esc(g)}</button>' for g in groups)+'<span class="counter"></span></nav>'
body=''.join(article(s) for s in m['scenarios'])
foot='''<section class="coverage"><h2>Coverage and interpretation</h2><p class="muted">This commit improves interaction and accessibility rather than redesigning the chat. Unchanged-looking pairs intentionally document the preserved appearance. Screen-reader-only semantics are listed beneath each pair.</p><table><thead><tr><th>Change</th><th>Comparisons</th></tr></thead><tbody>'''
foot+=''.join(f'<tr><td>{esc(a)}</td><td>{esc(b)}</td></tr>' for a,b in coverage)
foot+='''</tbody></table><p class="muted">Internal-only changes — shared close/showing/focus helpers, private replay and late-secret handling, signed-fixture and test refactors, visibility narrowing, and documentation — have no separate static visual difference. Race and cancellation guarantees are covered by the recorded browser/native tests and mutation checks, not inferred from still images.</p><details><summary>Reproduction and build provenance</summary><p>Fresh static example-data/no-sync builds of both commits, separate unused loopback ports, exact bundle hashes in builds.json. No Freenet node or live user data. Each isolated browser context receives the same deterministic fixture entropy and clock; wall-clock timers still run normally. Fixture hooks prepare states; displayed controls and actions are real application UI. Capture script: capture.cjs. Full machine-readable screenshot state: manifest.json.</p><pre>'''+esc(json.dumps(builds,indent=2))+'''</pre></details></section></main><script>const buttons=[...document.querySelectorAll('[data-filter]')],cards=[...document.querySelectorAll('article')];function select(g){for(const b of buttons)b.classList.toggle('active',b.dataset.filter===g);for(const c of cards)c.hidden=g!=='All'&&c.dataset.group!==g;document.querySelector('.counter').textContent=cards.filter(c=>!c.hidden).length+' comparisons'}buttons.forEach(b=>b.onclick=()=>select(b.dataset.filter));select('All')</script></html>'''
(root/'index.html').write_text(head+body+foot)
# A complete, directly viewable inline gallery if it fits Orca's limit;
# otherwise show the primary changed interactions and link to the full gallery.
scratch=Path(os.environ['ORCA_CHAT_SCRATCH_DIR'])
inlinebody=''.join(article(s,True) for s in m['scenarios'])
inlinehead=f'<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><style>{css}</style><main><div class="kicker">Actual parent vs finished build</div><h1>Reaction actions: before and after</h1>'
inlinepage=inlinehead+inlinebody+'</main></html>'
if len(inlinepage.encode())>510000:
 chosen={'02-keyboard-enter','05-list-switch','07-phone-tap','10-standing-order'}
 inlinepage=inlinehead+'<p class="muted">Four highlighted changes. The linked full gallery contains every comparison and the full-window screenshots.</p>'+''.join(article(s,True) for s in m['scenarios'] if s['id'] in chosen)+'</main></html>'
name='river-reaction-comparison-b5ea6bcb.html'
assert len(inlinepage.encode())<512000
(scratch/name).write_text(inlinepage)
# Portable offline evidence pack, excluding the archive itself.
archive=root/'river-reaction-before-after.zip'
with zipfile.ZipFile(archive,'w',compression=zipfile.ZIP_DEFLATED) as z:
 for p in root.rglob('*'):
  if p.is_file() and p!=archive and p.suffix in {'.png','.json','.html','.cjs','.py'}:z.write(p,p.relative_to(root))
print(json.dumps({'gallery':str(root/'index.html'),'archive':str(archive),'inline':str(scratch/name),'inlineBytes':len(inlinepage.encode()),'pairs':len(m['scenarios'])},indent=2))
