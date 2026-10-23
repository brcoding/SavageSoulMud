"""Merge authored development rooms into current live world; never execute LPC.

Defaults read the current installation as the merge base, snapshot it once, and
write reviewable repository artifacts only. Re-running preserves existing rooms.
"""
from pathlib import Path
import argparse, collections, hashlib, json, re, shutil
ROOT=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser();parser.add_argument('--source',type=Path,default=Path(r'C:\Users\barry\OneDrive\Documents\37devbackup\ds3.7a7-dev\lib'));parser.add_argument('--base',type=Path,default=Path('P:/savsoulmud/grim'));args=parser.parse_args();LIB=args.source
ns={'__file__':str(ROOT/'migration/convert_population.py')};exec((ROOT/'migration/convert_population.py').read_text(encoding='utf-8').split('sources={}')[0],ns)
calls,strings,clean,uncomment=[ns[x] for x in ('calls','strings','clean','uncomment')]
report={'source':str(LIB),'base':str(args.base),'added_rooms':[],'merged_exits':[],'excluded':[],'unresolved_exits':[],'unsupported_callbacks':[],'source_files':{},'adaptations':[]}
archive=ROOT/'migration/development-source';backup=ROOT.parents[1]/'work/dev-import-backup'
def write(p,x):p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(x,indent=2)+'\n',encoding='utf-8')
def source(path):
 p=LIB/path.lstrip('/');p=p if p.suffix else p.with_suffix('.c')
 if not p.is_file():return ''
 d=archive/p.relative_to(LIB);d.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,d);report['source_files'][p.relative_to(LIB).as_posix()]=hashlib.sha256(p.read_bytes()).hexdigest()
 return uncomment(p.read_text(encoding='utf-8',errors='replace'))
def value(s,k):
 found=calls(s,k)
 if not found:return ''
 # A callback is not a literal description.
 return clean(''.join(strings(found[-1]))) if not found[-1].lstrip().startswith('(:') else ''
def ident(path):return hashlib.sha256(('development:'+path.lower().removesuffix('.c')).encode()).hexdigest()[:12]
def inventory(s):
 out=[]
 for expr in calls(s,'SetInventory'):
  for m in re.finditer(r'"(/[^"\n]+)"\s*:\s*(\(\{\s*\d+\s*,\s*\d+\s*\}\)|\d+|"(?:\\.|[^"\\])*")',expr):
   nums=[int(n) for n in re.findall(r'\d+',m[2])];delay=nums[0] if m[2].startswith('({') else 60;qty=nums[1] if m[2].startswith('({') else nums[0] if nums else 1
   out.append((m[1].removesuffix('.c'),max(1,min(qty,100)),delay))
 return out
def fixture(s,path):
 name=value(s,'SetKeyName') or Path(path).stem.replace('_',' ')
 keys=[name]
 for e in calls(s,'SetId'):keys+=strings(e)
 if name=='button' and keys[1:]:name=keys[1]
 if '/crafting/tools/' in path:keys+=['craft:'+Path(path).stem.replace('_','-')]
 return {'name':name,'keywords':list(dict.fromkeys(keys)),'description':[value(s,'SetLong') or value(s,'SetShort') or name],'room_description':value(s,'SetShort')+'.'}
def scenery(s,path):
 out=[]
 for expr in calls(s,'SetItems'):
  for m in re.finditer(r'(\(\{.*?\}\)|"(?:\\.|[^"\\])*")\s*:\s*((?:"(?:\\.|[^"\\])*"\s*\+?\s*)+)',expr,re.S):
   keys=strings(m[1]);out.append({'name':keys[0],'keywords':keys,'description':[clean(''.join(strings(m[2])))]})
 for expr in calls(s,'SetRead'):
  split=re.match(r'\s*(\(\{.*?\}\)|"(?:\\.|[^"\\])*")\s*,\s*(.*)',expr,re.S)
  if not split:continue
  keys=strings(split[1]);text=''
  if split[2].lstrip().startswith('"'):text=clean(''.join(strings(split[2])))
  else:
   callback=re.search(r'\(:\s*(\w+)\s*:\)',split[2]);body=re.search(r'\b'+callback[1]+r'\s*\([^)]*\)\s*\{(.*?)\}',s,re.S) if callback else None
   if body:
    heredoc=re.search(r'@([A-Za-z]+)\s*\n(.*?)\n\1',body[1],re.S)
    if heredoc:text=clean(heredoc[2])
    for p in strings(body[1]):
     if p.startswith('/domains/') and (LIB/p.lstrip('/')).is_file():source(p);text=clean((LIB/p.lstrip('/')).read_text(errors='replace'));break
  if not text:report['unsupported_callbacks'].append({'source':path,'read':expr});continue
  found=next((o for o in out if any(k in o['keywords'] for k in keys)),None)
  if found:found['description']+=[text]
  else:out.append({'name':keys[0],'keywords':keys,'description':[text]})
 return out
area=json.loads((args.base/'data/areas/savsoul.json').read_text(encoding='utf-8'))
for rel in ['data/areas/savsoul.json','data/savage-soul/npc-profiles.json']:
 p=args.base/rel;d=backup/rel
 if p.exists() and not d.exists():d.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,d)
areas={'savsoul':area,'southfield':{'id':ident('area:southfield'),'slug':'southfield','name':'Southfield','canonical':True,'rooms':[]}}
# Alias the already-restored medical hub, never create a duplicate.
old={r['slug']:r for r in area['rooms']};lookup={};sources={};new=set()
exclude={'death','freezer','furnace','pod','void','wiz_hall','romedcenterstorage','ronmetalswillerstorage','rotradersamsstorage','rustystorage','metalheadstorage','betweenthewalls'}
for domain in ('savsoul','southfield'):
 for p in sorted((LIB/'domains'/domain/'room').glob('*.c')):
  key='/'+p.relative_to(LIB).as_posix().removesuffix('.c');slug=p.stem.lower()
  if slug in exclude or (domain=='southfield' and slug=='start'):report['excluded'].append(key);continue
  existing=old.get('medical-center-hub' if slug=='medical_center_hub' else slug) if domain=='savsoul' else None
  if existing:lookup[key.lower()]=existing;continue
  s=source(key);r={'id':ident(key),'slug':slug,'name':value(s,'SetShort'),'description':value(s,'SetDayLong') or value(s,'SetLong'),'exits':{},'scenery':scenery(s,key),'npcs':[],'objects':[]}
  assert r['name'] and r['description'],key
  lookup[key.lower()]=r;sources[key]=(s,r);new.add(r['id']);areas[domain]['rooms'].append(r);report['added_rooms'].append({'source':key,'id':r['id'],'slug':slug})
# Finite native projection of the route needed for the new authored Southfield shops.
source('/domains/savsoul/virtual/ROWasteland.c')
waste={'id':ident('area:royaloak-wasteland'),'slug':'royaloak-wasteland','name':'Royal Oak Wastelands','canonical':True,'rooms':[]};areas['royaloak-wasteland']=waste
vectors={'north':(0,1),'northeast':(1,1),'east':(1,0),'southeast':(1,-1),'south':(0,-1),'southwest':(-1,-1),'west':(-1,0),'northwest':(-1,1)}
for x in range(2,5):
 for y in range(-14,1):
  path=f'/domains/savsoul/virtual/ROWasteland/{x},{y}';desc='The wastelands spread out from here offering little solstice as the grit blows about the area in all direction.'
  if x==2 and y!=0:desc+=' A large pile of garbage prevents further travel west.'
  if y==0 and x!=4:desc+=' A large pile of garbage prevents further travel north.'
  if (x,y)==(2,-14):desc+=' A path west leads into East Southfield.'
  r={'id':ident(path),'slug':f'wasteland-{x}-{abs(y)}','name':'Wastelands','description':desc,'exits':{},'scenery':[{'name':'wasteland','keywords':['wasteland'],'description':['A place of violent death and great destruction.']} ]}
  lookup[path.lower()]=r;waste['rooms'].append(r);new.add(r['id'])
for x in range(2,5):
 for y in range(-14,1):
  r=lookup[f'/domains/savsoul/virtual/rowasteland/{x},{y}']
  for d,(dx,dy) in vectors.items():
   if 2<=x+dx<=4 and -14<=y+dy<=0:r['exits'][d]=lookup[f'/domains/savsoul/virtual/rowasteland/{x+dx},{y+dy}']['id']
  if (x,y)==(4,0):r['exits']['north']=old['rosouthroad17']['id']
  if (x,y)==(2,-14):r['exits']['west']=lookup['/domains/southfield/room/southfieldcityeast01']['id']
report['adaptations']+=['Materialized only x=2..4, y=-14..0 (45 cells) of the source wasteland generator, the playable corridor between Royal Oak and Southfield. Outer generated wilderness is not imported.','Repaired missing west exit from wasteland 2,-14, implied by the authored Southfield east exit.','Existing room descriptions, IDs, NPCs and exits are preserved; only missing source exits into new rooms are merged.','Source container contents lie beside fixed containers because Grim does not yet model nested inventories.','The ATM remains scenery: legacy banking and PvP arena are not implemented.']
for key,(s,r) in sources.items():
 for expr in calls(s,'SetExits'):
  for d,t in re.findall(r'"(\w+)"\s*:\s*"([^"\n]+)"',expr):
   dest=lookup.get(t.removesuffix('.c').lower())
   if dest:r['exits'][d]=dest['id']
   else:report['unresolved_exits'].append({'room':key,'direction':d,'target':t})
 for path,qty,delay in inventory(s):
  text=source(path)
  if not text:continue
  if '/npc/' in path and 'LIB_TELLER' not in text:
   n=fixture(text,path);n.update(race=value(text,'SetRace'),gender=value(text,'SetGender'));level=calls(text,'SetLevel');n['level']=int(level[-1]) if level and level[-1].strip().isdigit() else 1
   wander=calls(text,'SetWanderSpeed');interval=int(wander[-1]) if wander and wander[-1].strip().isdigit() else 15
   n['spawn']={'max_alive':qty,'respawn_seconds':delay,'movement':{'mode':'wander' if wander else 'stay','interval_seconds':interval,'max_distance':2,'same_area':True}}
   r['npcs'].append(n)
  else:
   item=fixture(text,path);fixed='CanGet' in text or 'LIB_TABLE' in text or 'LIB_TELLER' in text
   target='scenery' if fixed else 'objects'
   for _ in range(qty):r[target].append(dict(item))
   if fixed:
    for child,count,_ in inventory(text):
     childtext=source(child)
     for _ in range(count):r['objects'].append(fixture(childtext,child))
 # Callbacks requiring runtime adapters are explicitly audited.
 if re.search(r'Set(?:Search|Listen|Smell)\s*\(.*?:',s):report['unsupported_callbacks'].append({'source':key,'kind':'dynamic search/listen/smell'})
# Existing rooms with restored NPCs keep them. Empty rooms gain development NPCs.
for p in sorted((LIB/'domains/savsoul/room').glob('*.c')):
 key='/'+p.relative_to(LIB).as_posix().removesuffix('.c');r=lookup.get(key.lower())
 if not r or r['id'] in new or r.get('npcs'):continue
 text=uncomment(p.read_text(encoding='utf-8',errors='replace'))
 for path,qty,delay in inventory(text):
  if '/npc/' not in path:continue
  mob=source(path)
  if not mob or 'LIB_TELLER' in mob:continue
  n=fixture(mob,path);n.update(race=value(mob,'SetRace'),gender=value(mob,'SetGender'));level=calls(mob,'SetLevel');n['level']=int(level[-1]) if level and level[-1].strip().isdigit() else 1
  n['spawn']={'max_alive':qty,'respawn_seconds':delay,'movement':{'mode':'stay'}}
  r.setdefault('npcs',[]).append(n);source(key)
  report.setdefault('existing_room_npcs_added',[]).append({'room':r['slug'],'name':n['name'],'source':path})
# New links must be reachable from the preserved streets, not a teleport concourse.
for p in sorted((LIB/'domains/savsoul/room').glob('*.c')):
 key='/'+p.relative_to(LIB).as_posix().removesuffix('.c');r=lookup.get(key.lower())
 if not r or r['id'] in new:continue
 s=uncomment(p.read_text(encoding='utf-8',errors='replace'))
 for expr in calls(s,'SetExits'):
  for d,t in re.findall(r'"(\w+)"\s*:\s*"([^"\n]+)"',expr):
   dest=lookup.get(t.removesuffix('.c').lower())
   if dest and dest['id'] in new and d not in r['exits']:
    r['exits'][d]=dest['id'];report['merged_exits'].append({'room':r['slug'],'direction':d,'target':dest['slug']});source(key)
# Bank source provides east->checkout but checkout omits its reciprocal.
bank=lookup.get('/domains/savsoul/room/bank');shop=old['rotradersams01']
if bank and 'west' not in shop['exits']:shop['exits']['west']=bank['id'];report['adaptations'].append('Restored west checkout->ATM link from its authored reverse exit; no legacy banking verbs.')
# Add native-readable lens state for the runtime adapter before first interaction.
machine=lookup['/domains/savsoul/room/morlockcavemachine01']
if not any('lenses' in item.get('keywords',[]) for item in machine['scenery']):machine['scenery'].append({'name':'lenses','keywords':['lens','lenses'],'description':['Four lenses point west, southeast, east and north. Colored buttons adjust their rotation.']})
notice='The sign preserves the old banking instructions. Banking services are not active in this port; credits remain in your character balance.'
bank_description=lookup['/domains/savsoul/room/bank']['scenery'][0]['description']
if notice not in bank_description:bank_description.append(notice)
# Keep new machines as scenery with deterministic per-tool identity aliases.
for a in areas.values():write(ROOT/'data/areas'/f"{a['slug']}.json",a)
allareas=[json.loads(p.read_text(encoding='utf-8')) for p in (ROOT/'data/areas').glob('*.json')];allrooms=[r for a in allareas if a.get('canonical') for r in a['rooms']];byid={r['id']:r for r in allrooms}
assert len(byid)==len(allrooms),'Duplicate room identity'
assert all(t in byid for r in allrooms for t in r.get('exits',{}).values()),'Dangling exit'
seen=set();q=collections.deque([area['starting_room']])
while q:
 id=q.popleft()
 if id in seen:continue
 seen.add(id);q.extend(byid[id].get('exits',{}).values())
report['new_rooms_unreachable']=[r['slug'] for r in allrooms if r['id'] in new and r['id'] not in seen]
assert not report['new_rooms_unreachable'],report['new_rooms_unreachable']
report['new_authored_rooms']=len(sources);report['new_generated_rooms']=len(waste['rooms']);report['new_npc_placements']=sum(len(r.get('npcs',[])) for _,r in sources.values());report['world_rooms']=len(allrooms);report['world_npc_blueprints']=sum(len(r.get('npcs',[])) for r in allrooms)
write(ROOT/'migration/development-room-import.json',report)
print(json.dumps({k:v for k,v in report.items() if k not in ('source_files','unsupported_callbacks','added_rooms','excluded')},indent=2))
