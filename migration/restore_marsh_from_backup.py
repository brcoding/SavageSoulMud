"""Recover the production marsh, wraith and Nurse Sue quest prerequisites."""
from pathlib import Path
import json,re,hashlib,shutil,collections
ROOT=Path(__file__).resolve().parents[1];LIB=Path(r'P:\savsoulmud\dev37prodback\ds3.7a7\lib');SRC=LIB/'domains/savsoul'
ns={'__file__':str(ROOT/'migration/convert_population.py')};exec((ROOT/'migration/convert_population.py').read_text(encoding='utf-8').split('sources={}')[0],ns)
calls,strings,clean,uncomment=[ns[x] for x in ('calls','strings','clean','uncomment')]
NPC_PROFILES=json.loads((ROOT/'data/savage-soul/npc-profiles.json').read_text(encoding='utf-8'))
def value(s,k):
 c=calls(s,k);return clean(''.join(strings(c[-1]))) if c else ''
def gid(k):return hashlib.sha256(('savsoul:'+k).encode()).hexdigest()[:12]
def source(rel):
 p=SRC/rel;d=ROOT/'migration/production-marsh-source'/rel;d.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,d);return uncomment(p.read_text())
def scenery(s):
 out=[]
 for expr in calls(s,'SetItems'):
  for m in re.finditer(r'(\(\{.*?\}\)|"(?:\\.|[^"\\])*")\s*:\s*((?:"(?:\\.|[^"\\])*"\s*\+?\s*)+)',expr,re.S):
   keys=strings(m[1]);out.append({'name':keys[0],'keywords':keys,'description':[clean(''.join(strings(m[2])))]})
 # Recover written text from static SetRead callbacks without executing LPC.
 for expr in calls(s,'SetRead'):
  callback=re.match(r'\s*"([^"\n]+)"\s*,\s*\(:\s*(\w+)\s*:\)',expr)
  if not callback: continue
  body=re.search(r'\b'+re.escape(callback[2])+r'\s*\([^)]*\)\s*\{(.*?)\}',s,re.S)
  if not body: continue
  for text_path in strings(body[1]):
   if not text_path.startswith('/domains/savsoul/txt/'): continue
   src=LIB/text_path.lstrip('/')
   if not src.is_file(): continue
   archive=ROOT/'migration/production-marsh-source'/src.relative_to(SRC)
   archive.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(src,archive)
   written=clean(src.read_text())
   for fixture in out:
    if callback[1] in fixture['keywords']:
     fixture['description']=[re.sub(r"\s*To read it, 'read sign'\.", '', d) for d in fixture['description']]+[written]
 return out
def npc(rel):
 s=source(rel);name=value(s,'SetKeyName');ids=[name]
 for expr in calls(s,'SetId'):ids+=strings(expr)
 level=calls(s,'SetLevel');result={'name':name,'keywords':list(dict.fromkeys(ids)),'description':[value(s,'SetLong')],'room_description':value(s,'SetShort')+' is here.','race':value(s,'SetRace'),'gender':value(s,'SetGender'),'level':int(level[-1]) if level and level[-1].strip().isdigit() else 1}
 result.update(NPC_PROFILES.get(name.lower(),{}));return result
p=ROOT/'data/areas/savsoul.json';area=json.loads(p.read_text(encoding='utf-8'));rooms={r['slug']:r for r in area['rooms']}
names=['FadedTrail%02d'%i for i in range(1,6)]+['ZombieMarsh%02d'%i for i in range(1,18)]
known={n.lower() for n in names}|rooms.keys();unresolved=[]
for name in names:
 s=source('room/'+name+'.c');slug=name.lower();room=rooms.get(slug)
 if room is None:room={'id':gid(name),'slug':slug};area['rooms'].append(room);rooms[slug]=room
 exits={}
 for expr in calls(s,'SetExits'):
  for d,t in re.findall(r'"(\w+)"\s*:\s*"([^"\n]+)"',expr):
   dest=Path(t).stem
   if t.startswith('/domains/savsoul/room/') and dest.lower() in known:exits[d]=gid(dest)
   else:unresolved.append({'room':name,'direction':d,'target':t})
 mobs=[]
 for expr in calls(s,'SetInventory'):
  for path in dict.fromkeys(re.findall(r'"(/domains/savsoul/npc/[^"\n]+)"',expr)):
   rel=path.removeprefix('/domains/savsoul/').removesuffix('.c')+'.c';mobs.append(npc(rel))
 room.update(name=value(s,'SetShort'),description=value(s,'SetDayLong') or value(s,'SetLong'),scenery=scenery(s),exits=exits,npcs=mobs)
# Restore the source's diagonal entrance and preserve the existing town topology.
main=rooms['romainst01'];main['exits'].pop('east',None);main['exits']['northeast']=gid('FadedTrail01')
hub=rooms['medical-center-hub'];hub.setdefault('npcs',[]);hub['npcs']=[n for n in hub['npcs'] if n['name'].lower()!='sue rodgers']+[npc('npc/suerodgers.c')]
source('obj/ROGateKey.c');source('obj/soulboy.c')
p.write_text(json.dumps(area,indent=2)+'\n',encoding='utf-8')
op=ROOT/'migration/savsoul-reconstruction.json';o=json.loads(op.read_text(encoding='utf-8'))
for slug in [n.lower() for n in names]+['romainst01','medical-center-hub']:o[slug]=dict(rooms[slug])
op.write_text(json.dumps(o,indent=2)+'\n',encoding='utf-8')
# Verify the accessible route and return before publishing static data.
byid={r['id']:r for r in area['rooms']};queue=collections.deque([('romainst01',[])]);seen=set();route=None
while queue:
 slug,path=queue.popleft()
 if slug in seen:continue
 seen.add(slug)
 if slug=='zombiemarsh07':route=path;break
 for d,id in rooms[slug]['exits'].items():
  if id in byid:queue.append((byid[id]['slug'],path+[d]))
assert route is not None
allareas=[json.loads(p.read_text(encoding='utf-8')) for p in (ROOT/'data/areas').glob('*.json')];allareas=[a for a in allareas if a.get('canonical')]
room_count=sum(len(a['rooms']) for a in allareas);mob_count=sum(len(r.get('npcs',[])) for a in allareas for r in a['rooms'])
report={'source':str(LIB),'route_from_south_main_street':route,'world_rooms':room_count,'world_npcs':mob_count,'unresolved':unresolved};(ROOT/'migration/marsh-import.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report))
