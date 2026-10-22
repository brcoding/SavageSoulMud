import pathlib, re, json, hashlib, shutil

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = pathlib.Path(r'P:\savsoulmud\lpmud\ds\ds3.6\lib\domains\savsoul')
archive = ROOT / 'migration' / 'savsoul-lpc'
shutil.copytree(SOURCE, archive, dirs_exist_ok=True)
def text(expr):
    return ''.join(json.loads(m.group()) for m in re.finditer(r'"(?:\\.|[^"\\])*"', expr))
def setting(src, name):
    m = re.search(r'\b'+name+r'\s*\((.*?)\);', src, re.S)
    return text(m[1]) if m else ''
def ident(key):
    return hashlib.sha256(('savsoul:'+key).encode()).hexdigest()[:12]
def slug(key):
    return re.sub(r'[^a-z0-9]+', '-', key.lower()).strip('-')
def clean(s):
    return re.sub(r'%\^[A-Z_]+%\^', '', s)
system = {'death','freezer','furnace','pod','void','wiz_hall'}
sources = {p.stem: p.read_text() for p in sorted((SOURCE/'room').glob('*.c'))}
rooms, review = [], {}
for key, src in sources.items():
    if key in system: continue
    long = setting(src, 'SetDayLong') or setting(src, 'SetLong')
    exits = {}
    for d,target in re.findall(r'"(\w+)"\s*:\s*"(/domains/[^" ]+)"', re.search(r'SetExits\s*\((.*?)\);',src,re.S)[1]):
        dest = pathlib.PurePosixPath(target).name.removesuffix('.c')
        if dest not in sources or dest in system: continue
        exits[{'northeast':'east','southwest':'west'}.get(d,d)] = ident(dest)
    if key == 'ROMainSt01': long = long.replace('northeast','east')
    if key == 'FadedTrail01': long += '\nThe trail returns west to South Royal Oak Main Street.'
    scenery=[]
    items = re.search(r'SetItems\s*\(\s*\(\[(.*?)\]\)', src, re.S)
    if items:
        pattern = r'(\(\{.*?\}\)|"(?:\\.|[^"\\])*")\s*:\s*("(?:\\.|[^"\\])*"(?:\s*\+\s*"(?:\\.|[^"\\])*")*)'
        for entry in re.finditer(pattern, items[1], re.S):
            keys = [json.loads(m.group()) for m in re.finditer(r'"(?:\\.|[^"\\])*"',entry[1])]
            desc = clean(text(entry[2])).replace("To read it, 'read sign'.", "Use 'read sign' or 'look sign' to read it.")
            if 'sign' in keys:
                sign = 'ROMedRoomSign.txt' if key=='start' else 'ROMedHubSign.txt'
                desc += '\n'+clean((SOURCE/'txt'/sign).read_text()).strip()
            scenery.append({'name':keys[0],'keywords':keys,'description':[desc]})
    if key == 'start':
        scenery.append({'name':'medical table','keywords':['table','surface','metal','medical table'],'description':[setting((SOURCE/'obj'/'med_table.c').read_text(),'SetLong')]})
        long += "\nUse 'look table' to examine the medical table and 'look sign' to read the sign."
    if key == 'Medical_Center_Hub': long += "\nUse 'read sign' or 'look sign' to read the medical center sign."
    rooms.append({'id':ident(key),'slug':slug(key),'name':setting(src,'SetShort'),'description':long,'exits':exits,'scenery':scenery})
    review[key]={'id':ident(key),'slug':slug(key),'climate':setting(src,'SetClimate'),'day':setting(src,'SetDayLong'),'night':setting(src,'SetNightLong'),'long':setting(src,'SetLong')}
overrides_file = ROOT/'migration'/'savsoul-reconstruction.json'
if overrides_file.exists():
    overrides = json.loads(overrides_file.read_text())
    for room in rooms:
        if room['slug'] in overrides: room.update(overrides[room['slug']])
    existing_slugs = {room['slug'] for room in rooms}
    for key, override in overrides.items():
        if key not in existing_slugs and all(field in override for field in ('id', 'slug', 'name', 'description', 'exits')):
            rooms.append(override.copy())
haven_file=ROOT/'data'/'areas'/'haven.json'
haven=json.loads(haven_file.read_text())
haven.pop('starting_room',None)
tavern=next(r for r in haven['rooms'] if r['slug']=='tavern')
gate=next(r for r in rooms if r['slug']==slug('ROMainStWallGate'))
door={'name':'the Savsoul door','keywords':['savsoul door','savsoul','portal'],'open':False}
tavern['exits']['west']=gate['id']
tavern['doors']['west']=door
portal_text = '\nA weathered Savsoul door stands to the west, opening onto Royal Oak.'
if portal_text not in tavern['description']: tavern['description'] += portal_text
gate['exits']['east']=tavern['id']
gate['doors']={'east':door}
gate['description'] += '\nA weathered Savsoul door to the east leads back to Haven.'
haven_file.write_text(json.dumps(haven,indent=2)+'\n')
(ROOT/'data'/'areas'/'savsoul.json').write_text(json.dumps({'id':ident('area'),'slug':'savsoul','name':'Savsoul: Royal Oak','canonical':True,'starting_room':ident('start'),'rooms':rooms},indent=2)+'\n')
(ROOT/'migration'/'room-map.json').write_text(json.dumps(review,indent=2)+'\n')
print(f'Converted {len(rooms)} public rooms; archived {len(system)} system rooms; source snapshot: {archive}')
