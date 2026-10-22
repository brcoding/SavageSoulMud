"""Convert static LPC world content; preserve unsupported source and report gaps."""
from pathlib import Path
import re, json, hashlib, collections, shutil

ROOT=Path(__file__).resolve().parents[1]
LIB=Path(r'C:\Users\barry\Documents\mudbackup\lib')
AREAS=['campus','town','cave','Ylsrim','Praxis']
S=re.compile(r'"(?:\\.|[^"\\])*"')
def strings(s):
    result=[]
    for m in S.finditer(s):
        try: result.append(json.loads(m.group()))
        except ValueError: result.append(m.group()[1:-1])
    return result
def clean(s): return re.sub(r'%\^[A-Za-z_]+%\^','',s).strip()
def uncomment(s):
    return re.sub(r'"(?:\\.|[^"\\])*"|/\*.*?\*/|//[^\n]*',lambda m:m[0] if m[0].startswith('"') else '',s,flags=re.S)
def calls(src,name):
    results=[]
    for m in re.finditer(r'\b'+re.escape(name)+r'\s*\(',src):
        start=m.end(); depth=1; quote=False; escape=False
        for i in range(start,len(src)):
            c=src[i]
            if quote:
                if escape: escape=False
                elif c=='\\': escape=True
                elif c=='"': quote=False
            elif c=='"': quote=True
            elif c=='(': depth+=1
            elif c==')':
                depth-=1
                if depth==0: results.append(src[start:i]); break
    return results
sources={}
for base in [LIB/'domains',LIB/'realms'/'hells',LIB/'secure'/'npc']:
    for p in sorted(base.rglob('*.c')):
        sources['/'+p.relative_to(LIB).as_posix().removesuffix('.c')]=uncomment(p.read_text(errors='replace'))
def resolve(path,owner=''):
    path=path.removesuffix('.c')
    if not path.startswith('/'): path=str(Path(owner).parent/path).replace('\\','/')
    return path
def chain(path,seen=None):
    seen=set() if seen is None else seen
    if path in seen or path not in sources: return []
    seen.add(path); result=[]
    for parent in re.findall(r'inherit\s+"([^"\n]+)"',sources[path]): result+=chain(resolve(parent,path),seen)
    return result+[sources[path]]
def value(src,name,default=''):
    found=calls(src,name)
    if not found: return default
    expr=found[-1]
    parts=strings(expr)
    if parts: return clean(''.join(parts))
    callback=re.search(r'\(:\s*(\w+)\s*:\)',expr)
    if callback:
        fn=re.search(r'\b'+callback[1]+r'\s*\([^)]*\)\s*\{(.*?)\n\}',src,re.S)
        if fn:
            static=re.search(r'(?:return|(?:ret|desc)\s*=)\s*((?:"(?:\\.|[^"\\])*"\s*\+?\s*)+)',fn[1])
            if static: return clean(''.join(strings(static[1])))
            file=re.search(r'read_file\("([^"\n]+)"\)',fn[1])
            if file and (LIB/file[1].lstrip('/')).is_file(): return clean((LIB/file[1].lstrip('/')).read_text(errors='replace'))
    return default
def data(path):
    return '\n'.join(chain(path))
def gid(path): return hashlib.sha256(('lpc:'+path).encode()).hexdigest()[:12]
def slug(path): return re.sub(r'[^a-z0-9]+','-',path.lower().strip('/'))
def roomslug(path,area): return slug(path.split('/'+area+'/',1)[-1])
def isroom(path,src):
    return ('/room/' in path or re.search(r'inherit\s+LIB_(?:ROOM|CHAPEL|STORAGE|FISHING|SHOP)',src) or calls(src,'SetExits')) and '/npc/' not in path
def isnpc(path):
    return '/npc/' in path or any(re.search(r'inherit\s+LIB_(?:NPC|SENTIENT|VENDOR|TRAINER|LEADER|MONSTER|BOT)',s) for s in chain(path))
macros={}
for p in (LIB/'include').rglob('*.h'):
    for key,val in re.findall(r'^\s*#define\s+(\w+)\s+"([^"\n]+)"',p.read_text(errors='replace'),re.M): macros[key]=val
def pairs(expr,owner):
    for m in re.finditer(r'"([^"\n]+)"\s*:\s*("[^"\n]+"|[A-Z][A-Z0-9_]+)',expr):
        token=m[2]; target=token[1:-1] if token.startswith('"') else macros.get(token,'')
        if target: yield m[1],resolve(target,owner)
rooms={}; notes=[]; npcs_by_source=collections.Counter()
for area in AREAS+['hells']:
    prefix='/realms/hells/' if area=='hells' else '/domains/'+area+'/'
    for path,source in sources.items():
        if not path.startswith(prefix) or not isroom(path,source): continue
        if any(x in path.lower() for x in ['/broken/','/attic/','/save/','/virtual/']): continue
        src=data(path)
        name=value(src,'SetShort')
        desc=value(src,'SetDayLong') or value(src,'SetLong') or value(src,'SetNightLong')
        if not name: notes.append({'source':path,'issue':'No static room name; retained in source archive'}); continue
        room={'id':gid(path),'slug':roomslug(path,area),'name':name,'description':desc or name+'.','exits':{},'npcs':[],'scenery':[],'source_path':path}
        rooms[path]=(area,room,src)
# Resolve literal exits only, recording gaps rather than wiring dangling refs.
directions={'north','south','east','west','up','down','northeast','northwest','southeast','southwest'}
for path,(area,r,src) in rooms.items():
    exits=calls(src,'SetExits')
    if exits:
        for direction,target in pairs(exits[-1],path):
            if target in rooms and direction.lower() in directions: r['exits'][direction.lower()]=gid(target)
            else: notes.append({'source':path,'issue':'Exit needs manual/virtual conversion','direction':direction,'target':target})
    for expr in calls(src,'AddExit'):
        parts=strings(expr)
        if len(parts)>=2 and parts[1] in directions and resolve(parts[0],path) in rooms: r['exits'][parts[1]]=gid(resolve(parts[0],path))
    # Inherited inventories are overridden by the final SetInventory call.
    inventory=calls(src,'SetInventory')
    placed={}
    if inventory:
        for m in re.finditer(r'"(/[^"\n]+)"\s*:\s*(\(\{[^}]*\}\)|\d+|"[^"\n]*")',inventory[-1]):
            target=resolve(m[1],path)
            if not isnpc(target) or target not in sources: continue
            nums=re.findall(r'\d+',m[2]); count=int(nums[-1]) if nums else 1
            placed[target]=max(1,min(count,30))
    # Explicit room-level dynamic NPC creation, e.g. Kim in bookstore init.
    for target in re.findall(r'(?:new|clone_object)\s*\(\s*"(/[^"\n]+)"',src):
        target=resolve(target,path)
        if target in sources and isnpc(target): placed.setdefault(target,1)
    for target,count in placed.items():
        nsrc=data(target); key=value(nsrc,'SetKeyName') or Path(target).name
        short=value(nsrc,'SetShort') or key
        description=value(nsrc,'SetLong') or short+'.'
        keywords=[key]
        for name in ['SetId','SetAdjectives']:
            c=calls(nsrc,name)
            if c: keywords+=strings(c[-1])
        level=calls(nsrc,'SetLevel'); n=re.search(r'\d+',level[-1]) if level else None
        npc={'name':short,'description':[description],'keywords':list(dict.fromkeys(keywords)),'room_description':short[0].upper()+short[1:]+' is here.','race':value(nsrc,'SetRace'),'level':int(n[0]) if n else 1,'gender':value(nsrc,'SetGender') or 'neutral','source_path':target}
        r['npcs'] += [npc.copy() for _ in range(count)]
        npcs_by_source[target]+=count
    items=calls(src,'SetItems')
    if items:
        for m in re.finditer(r'(\(\{.*?\}\)|"(?:\\.|[^"\\])*")\s*:\s*((?:"(?:\\.|[^"\\])*"\s*\+?\s*)+)',items[-1],re.S):
            keys=strings(m[1]); desc=clean(''.join(strings(m[2])))
            if keys and desc: r['scenery'].append({'name':keys[0],'keywords':keys,'description':[desc]})
# Existing IDs must remain stable for persisted Grim player locations.
savsoul_file=ROOT/'data/areas/savsoul.json'
savsoul=json.loads(savsoul_file.read_text())
med=next(r for r in savsoul['rooms'] if r['slug']=='start')
campus=rooms['/domains/campus/room/start'][1]
med['exits']['down']=campus['id']; campus['exits']['up']=med['id']
med['description'] += '\nA stairway down leads to the LPC University campus.' if 'LPC University campus' not in med['description'] else ''
campus['description'] += '\nA stairway up returns to the Royal Oak medical room.'
savsoul_file.write_text(json.dumps(savsoul,indent=2)+'\n',newline='\n')
# A travel concourse makes imported regions and otherwise disconnected static
# components accessible without replacing any authored direction.
hub={'id':gid('travel-concourse'),'slug':'travel-concourse','name':'Travel Concourse','description':'A long concourse connects the university to distant settlements. Marked archways lead to the areas shown below.','exits':{},'npcs':[]}
assert 'west' not in campus['exits']
campus['exits']['west']=hub['id']; hub['exits']['east']=campus['id']
allrooms={r['id']:r for _,r,_ in rooms.values()}
allrooms.update({r['id']:r for r in savsoul['rooms']}); allrooms[hub['id']]=hub
for existing in (ROOT/'data/areas').glob('*.json'):
    if existing.name.startswith('lpc-') or existing.name == 'savsoul.json': continue
    allrooms.update({r['id']:r for r in json.loads(existing.read_text())['rooms']})
def reachable(start):
    seen=set(); todo=[start]
    while todo:
        rid=todo.pop()
        if rid in seen: continue
        seen.add(rid); todo+=list(allrooms[rid]['exits'].values())
    return seen
spurs=[]; current=hub
for area in AREAS+['hells']:
    for path,(_,r,_) in rooms.items():
        if rooms[path][0]!=area or r['id'] in reachable(campus['id']): continue
        free=next((d for d in ['up','down','northwest','northeast','southwest','southeast','west','east','south','north'] if d not in r['exits']),None)
        if not free: notes.append({'source':path,'issue':'No free direction for concourse connection'}); continue
        node={'id':gid('concourse:'+path),'slug':'concourse-'+str(len(spurs)+1),'name':area.title()+' Archway','description':'An archway marked '+r['name']+' leads west into '+area+'. The concourse continues north and returns south.','exits':{'south':current['id'],'west':r['id']},'npcs':[]}
        opposite={'up':'down','down':'up','north':'south','south':'north','east':'west','west':'east','northeast':'southwest','southwest':'northeast','northwest':'southeast','southeast':'northwest'}
        # Portal direction need not match geometric inverse; return directions
        # are explicitly stated so no existing LPC exit is overwritten.
        r['exits'][free]=node['id']
        r['description']+='\nA marked archway '+free+' returns to the Travel Concourse.'
        current['exits']['north']=node['id']; spurs.append(node); allrooms[node['id']]=node; current=node
        node['description']='An archway marked '+r['name']+' leads west into '+area+'. The concourse continues north and returns south.'
        # Existing directed topology may be one-way; every static room gets a
        # return path, added only when its original topology has none.
    reached=reachable(campus['id'])
# Attach unreachable rooms conservatively; verify every NPC placement reachable.
for path,(_,r,_) in rooms.items():
    if hub['id'] in reachable(r['id']): continue
    free=next((d for d in ['up','down','northwest','northeast','southwest','southeast','west','east','south','north'] if d not in r['exits']),None)
    if free:
        r['exits'][free]=hub['id']
        r['description']+='\nA marked archway '+free+' returns to the Travel Concourse.'
    else: notes.append({'source':path,'issue':'No free direction for return path'})
assert all(r['id'] in reachable(campus['id']) for _,r,_ in rooms.values())
assert all(hub['id'] in reachable(r['id']) for _,r,_ in rooms.values())
for area in AREAS+['hells']:
    group=[r for a,r,_ in rooms.values() if a==area]
    if not group: continue
    bp={'id':gid('area:'+area),'slug':'lpc-'+area.lower(),'name':area.title(),'canonical':True,'rooms':group}
    (ROOT/'data/areas'/('lpc-'+area.lower()+'.json')).write_text(json.dumps(bp,indent=2)+'\n',newline='\n')
(ROOT/'data/areas/lpc-travel.json').write_text(json.dumps({'id':gid('area:travel'),'slug':'lpc-travel','name':'Travel Concourse','canonical':True,'rooms':[hub]+spurs},indent=2)+'\n',newline='\n')
archive=ROOT/'migration/current-lpc'
for area in AREAS:
    shutil.copytree(LIB/'domains'/area,archive/'domains'/area,dirs_exist_ok=True)
shutil.copytree(LIB/'realms/hells',archive/'realms/hells',dirs_exist_ok=True)
report={'source':str(LIB),'areas':{a:{'rooms':sum(x[0]==a for x in rooms.values()),'npc_instances':sum(len(x[1]['npcs']) for x in rooms.values() if x[0]==a)} for a in AREAS+['hells']},'npc_definitions_placed':len(npcs_by_source),'npc_instances':sum(npcs_by_source.values()),'concourse_rooms':1+len(spurs),'gaps':notes}
(ROOT/'migration/population-report.json').write_text(json.dumps(report,indent=2)+'\n',newline='\n')
print(json.dumps({k:v for k,v in report.items() if k!='gaps'},indent=2))
