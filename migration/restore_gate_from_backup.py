"""Recover static southern gate/road topology; report unported boundaries."""
from pathlib import Path
import json,re,hashlib,shutil
ROOT=Path(__file__).resolve().parents[1]
LIB=Path(r'P:\savsoulmud\dev37prodback\ds3.7a7\lib')
SOURCE=LIB/'domains/savsoul'
ns={'__file__':str(ROOT/'migration/convert_population.py')};exec((ROOT/'migration/convert_population.py').read_text(encoding='utf-8').split('sources={}')[0],ns)
calls,strings,clean,uncomment=[ns[k] for k in ('calls','strings','clean','uncomment')]
def value(s,name):
 c=calls(s,name);return clean(''.join(strings(c[-1]))) if c else ''
def gid(key):return hashlib.sha256(('savsoul:'+key).encode()).hexdigest()[:12]
def scenery(s):
 out=[]
 for expr in calls(s,'SetItems'):
  for m in re.finditer(r'(\(\{.*?\}\)|"(?:\\.|[^"\\])*")\s*:\s*((?:"(?:\\.|[^"\\])*"\s*\+?\s*)+)',expr,re.S):
   keys=strings(m[1]);out.append({'name':keys[0],'keywords':keys,'description':[clean(''.join(strings(m[2])))]})
 return out
p=ROOT/'data/areas/savsoul.json';area=json.loads(p.read_text(encoding='utf-8'));rooms={r['slug']:r for r in area['rooms']}
names=['ROMainStWallGate','ROGateSystem']+['ROSouthRoad%02d'%i for i in range(1,18)]+['ROChargingStation01','ROChargingStation02','SmallWhiteShack']
available={name.lower() for name in names}|rooms.keys();missing=[]
for name in names:
 src=SOURCE/'room'/f'{name}.c';s=uncomment(src.read_text());slug=name.lower();old=rooms.get(slug,{})
 archive=ROOT/'migration/production-gate-source/room';archive.mkdir(parents=True,exist_ok=True);shutil.copy2(src,archive/src.name)
 exits={}
 for expr in calls(s,'SetExits'):
  for d,target in re.findall(r'"(\w+)"\s*:\s*"([^"\n]+)"',expr):
   stem=Path(target).stem
   if target.startswith('/domains/savsoul/room/') and stem.lower() in available:exits[d]=gid(stem)
   else:missing.append({'room':name,'direction':d,'target':target})
 if name=='ROMainStWallGate':exits['south']=gid('ROSouthRoad01')
 fields={'name':value(s,'SetShort'),'description':value(s,'SetDayLong') or value(s,'SetLong'),'exits':exits,'scenery':scenery(s)}
 if name=='ROMainStWallGate':
  if 'east' in old.get('exits',{}):fields['exits']['east']=old['exits']['east'];fields['description']+='\nA weathered Savsoul door to the east leads back to Haven.'
  fields['doors']=dict(old.get('doors',{}));fields['doors']['south']={'name':'the south city gate','keywords':['gate','city gate','south gate'],'open':False}
 if name=='ROSouthRoad01':fields['doors']={'north':{'name':'the south city gate','keywords':['gate','city gate','north gate'],'open':False}}
 if name=='ROGateSystem':
  fields['scenery']=[x for x in fields['scenery'] if 'terminal' not in x['keywords']]
  fields['scenery'].append({'name':'Gate Terminal','keywords':['terminal','interface','computer','screen','display'],'description':['The terminal interface is a small keyboard affixed to the wall with large cables leading up along the wall. The gate terminal glows with a faint orange screen. The original terminal override interaction has not been ported yet.']})
 if not old:old={'id':gid(name),'slug':slug};area['rooms'].append(old)
 old.update(fields);rooms[slug]=old
p.write_text(json.dumps(area,indent=2)+'\n',encoding='utf-8')
overrides_file=ROOT/'migration/savsoul-reconstruction.json'
overrides=json.loads(overrides_file.read_text(encoding='utf-8'))
for name in names: overrides[name.lower()]=dict(rooms[name.lower()])
overrides_file.write_text(json.dumps(overrides,indent=2)+'\n',encoding='utf-8')
(ROOT/'migration/gate-boundaries.json').write_text(json.dumps(missing,indent=2)+'\n')
print('Recovered',len(names),'gate/control/road blueprints;',len(area['rooms']),'Savsoul rooms. Unported boundary exits:',len(missing))
