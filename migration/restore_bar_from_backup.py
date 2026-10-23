"""Restore source-authored Bar N Grill static content from the production backup.
This does not translate LPC callbacks, traps, vendors, boards or quest logic.
"""
from pathlib import Path
import json,re,hashlib,shutil
ROOT=Path(__file__).resolve().parents[1]
LIB=Path(r'P:\savsoulmud\dev37prodback\ds3.7a7\lib')
SOURCE=LIB/'domains/savsoul'
ARCHIVE=ROOT/'migration'/'production-bar-source'
# Reuse the existing balanced LPC call/string readers without running its importer.
helpers=(ROOT/'migration/convert_population.py').read_text().split('sources={}')[0]
ns={'__file__':str(ROOT/'migration/convert_population.py')};exec(helpers,ns)
calls,strings,clean,uncomment=[ns[k] for k in ('calls','strings','clean','uncomment')]
def source(rel):
 p=SOURCE/rel; dest=ARCHIVE/rel;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,dest)
 return uncomment(p.read_text(errors='replace'))
def value(s,key):
 found=calls(s,key)
 return clean(''.join(strings(found[-1]))) if found else ''
def item(s):
 name=value(s,'SetShort') or value(s,'SetKeyName')
 keys=[value(s,'SetKeyName')]
 for expr in calls(s,'SetId'):keys+=strings(expr)
 return {'name':name,'keywords':list(dict.fromkeys(k for k in keys if k)),'description':[value(s,'SetLong')]}
def scenery(s):
 result=[]
 for expr in calls(s,'SetItems'):
  pattern=r'(\(\{.*?\}\)|"(?:\\.|[^"\\])*")\s*:\s*((?:"(?:\\.|[^"\\])*"\s*\+?\s*)+)'
  for m in re.finditer(pattern,expr,re.S):
   keys=strings(m[1]);result.append({'name':keys[0],'keywords':keys,'description':[clean(''.join(strings(m[2])))]})
 return result
def ident(stem):return hashlib.sha256(('savsoul:'+stem).encode()).hexdigest()[:12]
p=ROOT/'data/areas/savsoul.json';a=json.loads(p.read_text());rooms={r['slug']:r for r in a['rooms']}
op=ROOT/'migration/savsoul-reconstruction.json';overrides=json.loads(op.read_text())
for path in sorted((SOURCE/'room').glob('ROBarNGrill*.c')):
 stem=path.stem;slug=stem.lower();s=source('room/'+path.name)
 fields={'name':value(s,'SetShort'),'description':value(s,'SetLong'),'scenery':scenery(s),'exits':{}}
 for expr in calls(s,'SetExits'):
  for direction,target in re.findall(r'"(\w+)"\s*:\s*"([^"]+)"',expr):
   fields['exits'][direction]=ident(Path(target).stem)
 room=rooms.get(slug)
 if room is None:room={'id':ident(stem),'slug':slug};a['rooms'].append(room)
 room.update(fields);overrides[slug]=dict(fields)
 if slug=='robarngrill01':
  rusty=source('npc/rusty.c');npc=item(rusty)
  npc.update(name=value(rusty,'SetKeyName').capitalize(),room_description='Rusty the barkeep is standing here.',race=value(rusty,'SetRace'),gender=value(rusty,'SetGender'),level=20)
  room['npcs']=[npc];room['scenery'].append({'name':'a digital forum','keywords':['digital','forum','forum board','screen'],'description':['A digital forum is here.']})
  overrides[slug]['npcs']=room['npcs']
 if slug=='robarngrill03':
  stove=LIB/'domains/crafting/tools/stove.c'
  if stove.exists():
   dest=ARCHIVE/'stove.c';shutil.copy2(stove,dest);room['scenery'].append(item(uncomment(stove.read_text(errors='replace'))))
 if slug=='robarngrillfl104':
  room['scenery']+=[item(source('obj/gbed.c')),item(source('obj/barngrilldesk.c'))]
  room['scenery']=[x for x in room['scenery'] if x['name']!='bed']
  # Older importer has no Fl104; retain complete blueprint for its override addition.
  overrides[slug].update(id=room['id'],slug=slug)
for rel in ['obj/daggernote.c','weap/dagger.c','obj/gateterminal.c','npc/wraith.c','npc/waltrodgers.c','npc/suerodgers.c','room/ROMainStWallGate.c','room/ROGateSystem.c','room/rustystorage.c']:
 source(rel)
source('txt/daggernote.txt')
p.write_text(json.dumps(a,indent=2)+'\n');op.write_text(json.dumps(overrides,indent=2)+'\n')
print('Restored 8 source-authored Bar N Grill rooms, scenery, Rusty, guest bed and desk.')
