"""Publish a public MMP projection; native Grim IDs remain canonical."""
from pathlib import Path
import argparse,json,os,collections,xml.etree.ElementTree as ET
VECTORS={'north':(0,2,0),'south':(0,-2,0),'east':(2,0,0),'west':(-2,0,0),'northeast':(2,2,0),'northwest':(-2,2,0),'southeast':(2,-2,0),'southwest':(-2,-2,0),'up':(0,0,1),'down':(0,0,-1),'in':(2,0,0),'out':(-2,0,0)}
def publish(root,output):
 root=Path(root);output=Path(output);registry=root/'data/mudlet-map-ids.json'
 areas=[json.loads(p.read_text(encoding='utf-8-sig')) for p in sorted((root/'data/areas').glob('*.json'))]
 areas=[a for a in areas if a.get('canonical') is True]
 rooms={};owner={}
 for area in areas:
  for room in area.get('rooms',[]):
   if room['id'] in rooms:raise ValueError('Duplicate canonical room ID')
   rooms[room['id']]=room;owner[room['id']]=area
 start=next(r['id'] for r in rooms.values() if owner[r['id']]['slug']=='savsoul' and r['slug']=='start')
 def exits(room):
  return {d:t for d,t in room.get('exits',{}).items() if d in VECTORS and t in rooms and not room.get('doors',{}).get(d,{}).get('hidden',False)}
 visible=set([start]);queue=collections.deque([start])
 while queue:
  for target in exits(rooms[queue.popleft()]).values():
   if target not in visible:visible.add(target);queue.append(target)
 index=json.loads(registry.read_text()) if registry.exists() else {'rooms':{},'areas':{}}
 for kind,keys in [('rooms',visible),('areas',{owner[r]['id'] for r in visible})]:
  mapping=index[kind];values=list(mapping.values())
  if len(set(values))!=len(values) or any(type(n)!=int or not 0<n<1000000 for n in values):raise ValueError('Invalid compact mapper registry')
  next_id=max(values,default=0)+1
  for key in sorted(keys):
   if key not in mapping:mapping[key]=next_id;next_id+=1
 positions={}
 for area in sorted(areas,key=lambda a:a['id']):
  ids=sorted(r for r in visible if owner[r]['id']==area['id']);adj={r:[] for r in ids}
  for r in ids:
   for direction,target in sorted(exits(rooms[r]).items()):
    if target in adj:
     delta=VECTORS[direction];adj[r].append((target,delta));adj[target].append((r,tuple(-v for v in delta)))
  used=set();offset=0
  for seed in ids:
   if seed in positions:continue
   positions[seed]=(offset,0,0);used.add(positions[seed]);queue=collections.deque([seed])
   while queue:
    r=queue.popleft();x,y,z=positions[r]
    for target,(dx,dy,dz) in sorted(adj[r]):
     if target in positions:continue
     point=(x+dx,y+dy,z+dz);nudge=0
     while point in used:
      nudge+=1;point=(x+dx+2*nudge,y+dy,z+dz)
     positions[target]=point;used.add(point);queue.append(target)
   offset=max(p[0] for p in used)+8
 doc=ET.Element('map');aa=ET.SubElement(doc,'areas');rr=ET.SubElement(doc,'rooms')
 for area in sorted(areas,key=lambda a:a['id']):
  if any(owner[r]['id']==area['id'] for r in visible):ET.SubElement(aa,'area',id=str(index['areas'][area['id']]),name=area['name'])
 for r in sorted(visible):
  room=rooms[r];node=ET.SubElement(rr,'room',id=str(index['rooms'][r]),area=str(index['areas'][owner[r]['id']]),title=room['name'],environment='1')
  x,y,z=positions[r];ET.SubElement(node,'coord',x=str(x),y=str(y),z=str(z))
  for direction,target in sorted(exits(room).items()):
   if target in visible:ET.SubElement(node,'exit',direction=direction,target=str(index['rooms'][target]))
 env=ET.SubElement(doc,'environments');ET.SubElement(env,'environment',id='1',name='Wasteland',color='3')
 ET.indent(doc,space='  ');xml=ET.tostring(doc,encoding='utf-8',xml_declaration=True)
 # Stage complete files first. Existing IDs never change or get recycled.
 output.parent.mkdir(parents=True,exist_ok=True);registry.parent.mkdir(parents=True,exist_ok=True)
 for path,data in [(registry,(json.dumps(index,indent=2)+'\n').encode()),(output,xml)]:
  pending=path.with_name(path.name+'.next')
  with pending.open('wb') as f:f.write(data);f.flush();os.fsync(f.fileno())
 os.replace(registry.with_name(registry.name+'.next'),registry)
 os.replace(output.with_name(output.name+'.next'),output)
 registry.chmod(0o600);output.chmod(0o644)
 print(f'Published {len(visible)} public rooms in {len(aa)} areas; hidden exits omitted.')
 return index
if __name__=='__main__':
 parser=argparse.ArgumentParser();parser.add_argument('--root',required=True);parser.add_argument('--output',required=True);args=parser.parse_args();publish(args.root,args.output)
