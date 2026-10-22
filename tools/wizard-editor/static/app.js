'use strict';
const $=id=>document.getElementById(id);
let csrf='',files=[],current=null,doc=null,selected=-1,mode='form',dirty=false,loading=false;
let restoringLocation=false;
let worldData=null,worldAt=0,worldFlight=null,mapView=null,mapDraftTimer=null;
const alphabet='0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
const id=()=>Array.from(crypto.getRandomValues(new Uint8Array(12)),n=>alphabet[n%62]).join('');
function el(tag,text,cls){const node=document.createElement(tag);if(text!==undefined)node.textContent=text;if(cls)node.className=cls;return node;}
function btn(text,action,cls){const node=el('button',text,cls);node.type='button';node.addEventListener('click',()=>Promise.resolve().then(action).catch(error=>status(error.message,true)));return node;}
function status(text,error=false){$('status').textContent=text;$('status').classList.toggle('error',error);}
function mark(){dirty=true;$('dirty').hidden=false;if(mode==='map'&&mapView){clearTimeout(mapDraftTimer);mapDraftTimer=setTimeout(()=>{mapDraftTimer=null;if(mode==='map')mapView.refreshDraft();},200);}}
function clean(){dirty=false;$('dirty').hidden=true;}
function confirmDiscard(){return !dirty||confirm('Discard your unsaved changes?');}
async function api(path,body){const response=await fetch('/api/'+path,{method:body===undefined?'GET':'POST',headers:body===undefined?{}:{'Content-Type':'application/json','X-CSRF-Token':csrf},body:body===undefined?undefined:JSON.stringify(body)});const result=await response.json();if(!response.ok){if(response.status===401||response.status===403){if(path!=='login')status('Your session may have expired. Reload and log in again.',true);}throw Error(result.error||'The request failed.');}return result;}
function guarded(action){return async()=>{try{await action();}catch(error){status(error.message,true);}};}
async function bootstrap(){const result=await api('session');csrf=result.csrf;if(result.authenticated)await enter(result.wizard);}
async function enter(wizard){$('login').hidden=true;$('workspace').hidden=false;$('wizard').textContent=wizard+' · Wizard';await loadFiles();await restoreLocation();}
async function loadFiles(){files=(await api('files')).files;renderFiles();}

// URLs contain navigation only; drafts, credentials and sessions stay private.
function locationState(){
 const state=new URLSearchParams();
 if(!current)return state;
 state.set('file',current.path);state.set('view',mode);
 if(isArea())state.set('room',selected>=0?doc.rooms[selected]?.id||'':'area');
 if(mode==='map'){state.set('scope',mapView.scope);state.set('style',mapView.style);state.set('floor',mapView.floor);}
 return state;
}
function syncLocation(replace=false){
 if(restoringLocation||!current||current.revision==='new')return;
 const hash='#'+locationState().toString();
 if(location.hash!==hash)window.history[replace||!location.hash?'replaceState':'pushState'](null,'',location.pathname+location.search+hash);
}
async function restoreLocation(){
 const route=new URLSearchParams(location.hash.slice(1)),path=route.get('file');
 if(!path)return;
 if(!files.some(file=>file.path===path)){status('The linked file is unavailable to this editor.',true);return;}
 restoringLocation=true;
 try{
  if(current?.path!==path){await openFile(path);if(current?.path!==path){return;}}
  if(mode==='raw')parseSource();
  const room=route.get('room');
  if(isArea()&&room){selected=room==='area'?-1:doc.rooms.findIndex(item=>item.id===room||item.slug===room);if(selected<0&&room!=='area')status('The linked room no longer exists. Showing area settings.',true);}
  const view=route.get('view');mode=['form','raw','preview','map'].includes(view)?view:doc?'form':'raw';
  if(mode==='map'&&!isArea())mode=doc?'form':'raw';
  if(!doc&&mode==='form')mode='raw';
  if(['area','world','nearby'].includes(route.get('scope')))mapView.scope=route.get('scope');
  if(['fantasy','battle','blueprint'].includes(route.get('style')))mapView.style=route.get('style');
  const floor=route.get('floor');mapView.floor=floor==='auto'||floor==='all'||/^-?\d+$/.test(floor||'')?floor:'auto';
  mapView.scopeInput.value=mapView.scope;mapView.styleInput.value=mapView.style;mapView.fitNext=true;
  $('source').value=doc?format(doc):current.text;render();
 }finally{restoringLocation=false;syncLocation(true);}
}
window.addEventListener('popstate',()=>restoreLocation().catch(error=>status(error.message,true)));

function renderFiles(){const target=$('files');target.replaceChildren();const query=$('file-search').value.toLowerCase();const category=$('category').value;files.filter(file=>(!category||file.category===category)&&file.path.toLowerCase().includes(query)).forEach(file=>{const button=btn('',()=>openFile(file.path),'file-button'+(current?.path===file.path?' selected':''));button.append(el('small',file.category),el('span',file.path.split('/').pop()));button.title=file.path;target.append(button);});}
function isArea(){return doc&&!Array.isArray(doc)&&Array.isArray(doc.rooms)&&current?.path.startsWith('data/areas/');}
function valueHere(){return isArea()&&selected>=0?doc.rooms[selected]:doc;}
function format(value){return JSON.stringify(value,null,2)+'\n';}
function content(){if(mode==='raw'||!doc)return $('source').value;return format(doc);}
function parseSource(){if(current?.path.endsWith('.json')){doc=JSON.parse($('source').value);if(isArea()&&selected>=doc.rooms.length)selected=-1;}else doc=null;}
async function openFile(path){if(loading||!confirmDiscard())return;loading=true;try{const file=await api('file?path='+encodeURIComponent(path));install(file);status('Loaded '+path);}finally{loading=false;}}
function install(file){current=file;doc=file.path.endsWith('.json')?JSON.parse(file.text):null;selected=isArea()&&doc.rooms.length?0:-1;mode=doc?'form':'raw';$('source').value=file.text;clean();render();}
function render(){if(!current)return;$('empty').hidden=true;$('file-title').textContent=current.path.split('/').pop();$('file-path').textContent=current.path;$('file-category').textContent=files.find(f=>f.path===current.path)?.category||'NEW CONTENT';$('activation').textContent=current.activation||'Save this draft, then apply world changes if needed.';$('records-panel').hidden=!isArea();$('editor-grid').classList.toggle('has-records',isArea());renderFiles();renderRecords();renderView();}
function renderRecords(){const target=$('records');target.replaceChildren();if(!isArea())return;target.append(btn('Area settings',()=>selectRecord(-1),'room-button'+(selected===-1?' selected':'')));const query=$('record-search').value.toLowerCase();doc.rooms.forEach((room,index)=>{const hay=JSON.stringify([room.name,room.slug,room.npcs?.map(n=>n.name),room.objects?.map(n=>n.name)]).toLowerCase();if(query&&!hay.includes(query))return;const button=btn('',()=>selectRecord(index),'room-button'+(selected===index?' selected':''));button.append(el('span',room.name||'Unnamed room'),el('small',(room.slug||room.id)+' · '+(room.npcs?.length||0)+' NPCs'));target.append(button);});}
function selectRecord(index){if(mode==='raw'){parseSource();mode='form';}selected=index;render();}
function renderView(){clearTimeout(mapDraftTimer);mapDraftTimer=null;document.body.classList.toggle('map-open',mode==='map');$('map-tab').disabled=!isArea();$('map-view').hidden=mode!=='map';$('records-panel').hidden=!isArea()||mode==='map';const json=!!doc;$('form-tab').disabled=!json;$('fields').hidden=mode!=='form';$('source').hidden=mode!=='raw';$('preview').hidden=mode!=='preview';for(const [name,view]of [['form','form'],['raw','raw'],['preview','preview'],['map','map']])$(name+'-tab').classList.toggle('active',mode===view);const tools=$('selection-tools');tools.replaceChildren();if(isArea()&&selected>=0){const room=doc.rooms[selected];tools.append(el('h2',room.name||'Room'));const actions=el('div',undefined,'actions');actions.append(btn('Duplicate',()=>{const clone=structuredClone(room);clone.id=id();clone.slug=(clone.slug||'room')+'-copy';clone.name+=' (copy)';doc.rooms.push(clone);selected=doc.rooms.length-1;mark();render();}),btn('Delete room',()=>{if(!confirm('Delete this room? You must remove or redirect any exits pointing to it before saving.'))return;doc.rooms.splice(selected,1);selected=-1;mark();render();},'quiet'));tools.append(actions);}else tools.append(el('h2',isArea()?'Area settings':mode==='raw'?'Document source':mode==='preview'?'Content preview':'Document fields'));
if(mode==='form'){const target=$('fields');target.replaceChildren();renderValue(target,valueHere(),[],next=>{if(isArea()&&selected>=0)doc.rooms[selected]=next;else doc=next;mark();},true);}else if(mode==='preview'){let value=valueHere();let text=typeof value==='string'?value:value?.description||(doc?current.text:$('source').value);if(Array.isArray(text))text=text.join('\n\n');if(typeof text!=='string')text=format(text);$('preview').textContent=text.replace(/\{[krgybmcwKRGYBMCWx]/g,'');}else if(mode==='map'){mapView.render();}syncLocation();}
function template(key){if(['npcs','objects','scenery','stock','triggers','rooms','keywords','description','aliases'].includes(key))return [];if(['open','hidden','canonical','same_area','sharp','wizard','can_be_trampled'].includes(key))return false;if(['max_alive','level','respawn_seconds','interval_seconds','max_distance','price','sell_price','buy_price','xp_reward'].includes(key))return 1;if(['exits','doors','spawn','movement','shop','soul','combat_behavior','hackable','condition','attribute_modifiers'].includes(key))return {};return '';}
function entryTemplate(key){if(key==='rooms')return{id:id(),slug:'new-room',name:'New room',description:'',exits:{}};if(['npcs','objects','scenery'].includes(key))return{name:key==='npcs'?'New creature':'New object',description:[''],keywords:[],room_description:''};if(key==='stock')return{item:{name:'New item',description:[''],keywords:[],room_description:''},buy_price:1,sell_price:0};if(key==='triggers')return{on:'',script:''};if(key==='description'||key==='keywords'||key==='aliases')return '';return '';}
function friendly(key){return key.replaceAll('_',' ').replace(/^./,c=>c.toUpperCase());}
function summary(value,key){return value?.name||value?.slug||value?.id||key;}
function renderValue(parent,value,path,assign,top=false){
 if(path.length===1&&path[0]==='exits'&&isArea()&&selected>=0){renderExitFields(parent,doc.rooms[selected]);return;}
 if(Array.isArray(value)){
  const key=path.at(-1)||'';value.forEach((item,index)=>{const row=el('div',undefined,'array-row');const body=el('div',undefined,'array-content');renderValue(body,item,[...path,index],next=>{value[index]=next;mark();});row.append(body,btn('×',()=>{if(typeof item==='object'&&!confirm('Remove this entry?'))return;value.splice(index,1);mark();renderView();},'quiet'));parent.append(row);});parent.append(btn('+ Add entry',()=>{let next=entryTemplate(key);if(!value.length&&!next&&key!=='description'&&key!=='keywords'&&key!=='aliases'){const kind=prompt('Entry type: text, object, number or boolean','text');if(kind===null)return;next=kind==='object'?{}:kind==='number'?0:kind==='boolean'?false:'';}if(value.length)next=typeof value[0]==='object'?structuredClone(value[0]):typeof value[0]==='number'?0:typeof value[0]==='boolean'?false:'';if(next&&typeof next==='object'&&next.id)next.id=id();value.push(next);mark();renderView();},'add-button'));return;
 }
 if(value!==null&&typeof value==='object'){
  let box=parent;
  if(!top){const details=el('details');const sum=el('summary',summary(value,friendly(String(path.at(-1)))));sum.append(el('small',Object.keys(value).length+' fields'));details.append(sum);const child=el('div',undefined,'detail-body');details.append(child);let made=false;details.addEventListener('toggle',()=>{if(details.open&&!made){made=true;objectFields(child,value,path);}});parent.append(details);return;}
  objectFields(box,value,path);return;
 }
 const key=path.at(-1);let input;
 if(typeof value==='boolean'){input=el('input');input.type='checkbox';input.checked=value;input.addEventListener('change',()=>assign(input.checked));}
 else if(typeof value==='number'){input=el('input');input.type='number';input.step=Number.isInteger(value)?'1':'any';input.value=value;input.addEventListener('change',()=>{if(input.value===''||!Number.isFinite(Number(input.value))){input.setCustomValidity('Enter a valid number.');return;}input.setCustomValidity('');assign(Number(input.value));mark();});}
 else if(value===null){input=el('textarea');input.value='null';input.setAttribute('aria-label','JSON value');input.addEventListener('change',()=>{try{assign(JSON.parse(input.value));input.setCustomValidity('');mark();}catch(e){input.setCustomValidity('Enter a JSON value or null.');}});}
 else{input=el(typeof value==='string'&&(value.includes('\n')||path.includes('description')||['description','room_description','script','text','actor','room','target'].includes(key))?'textarea':'input');input.value=value??'';if(input.tagName==='TEXTAREA')input.rows=Math.min(12,Math.max(3,String(value).split('\n').length+1));if(key==='id'){input.readOnly=true;input.title='Stable identity; generated once and referenced by exits.';}input.addEventListener('input',()=>{assign(input.value);mark();});}
 input.setAttribute('aria-label',path.map(String).join(' / ')||'Value');parent.append(input);
}
function objectFields(parent,obj,path){const keys=Object.keys(obj);if(mode==='map'&&path.length===0&&isArea()&&selected>=0){const order=['name','slug','description','exits','doors','npcs','objects','scenery'];keys.sort((a,b)=>(order.includes(a)?order.indexOf(a):order.length)-(order.includes(b)?order.indexOf(b):order.length));}for(const key of keys){if(topAreaRooms(path,key))continue;const field=el('div',undefined,'field');const label=el('label',friendly(key));const holder=el('div');field.append(label,holder);renderValue(holder,obj[key],[...path,key],next=>{obj[key]=next;mark();});if(key!=='id')field.append(btn('Remove field',()=>{if(!confirm('Remove '+key+'?'))return;delete obj[key];mark();renderView();},'quiet'));parent.append(field);}const tools=el('div',undefined,'object-tools');tools.append(btn('+ Add field',()=>{const key=prompt('Field name (for example: npcs, objects, scenery, doors, spawn, shop or combat_behavior)');if(!key)return;if(['__proto__','prototype','constructor'].includes(key)||Object.hasOwn(obj,key))throw Error('Choose a new field name.');obj[key]=template(key);mark();renderView();},'add-button'));parent.append(tools);}
function topAreaRooms(path,key){return isArea()&&selected===-1&&path.length===0&&key==='rooms';}
async function switchMode(next){if(!current)return;if(next==='map'&&!isArea())throw Error('Open an area file to use the map.');if(next==='map'&&mode!=='map')mapView.fitNext=true;if(mode==='raw'&&next!=='raw')parseSource();if(next==='raw'&&mode!=='raw')$('source').value=doc?format(doc):$('source').value;mode=next;renderRecords();renderView();}
function validInputs(){const target=mode==='map'?mapView.inspector:mode==='form'?$('fields'):null;if(target)for(const input of target.querySelectorAll('input,textarea,select'))if(!input.reportValidity())throw Error('Correct the highlighted field first.');}
async function save(){if(!current)return;validInputs();const text=content();if(current.path.endsWith('.json'))JSON.parse(text);$('save').disabled=true;try{const result=await api('save',{path:current.path,revision:current.revision,text});const prev=selected,view=mode;current=result;doc=current.path.endsWith('.json')?JSON.parse(result.text):null;selected=prev;mode=view;clean();worldData=null;$('source').value=result.text;await loadFiles();render();status('Saved. '+result.activation+(result.warnings?.length?'\nExisting world warnings: '+result.warnings.join('; '):''));}finally{$('save').disabled=false;}}
async function showHistory(){if(!current)return;const result=await api('history?path='+encodeURIComponent(current.path));const versions=$('versions');versions.replaceChildren();if(!result.versions.length)versions.append(el('p','No previous versions yet. A backup is made each time you save an existing file.'));for(const version of result.versions){const row=el('div',undefined,'version');const info=el('div',new Date(version.at).toLocaleString());info.append(el('small','Saved by '+version.wizard));row.append(info,btn('Restore',async()=>{if(!confirm('Restore this version? Your current saved file will be backed up. Unsaved changes will be discarded.'))return;const result=await api('restore',{path:current.path,version:version.id,revision:current.revision});install(result);$('history-dialog').close();status('Previous version restored. '+result.activation);}));versions.append(row);}$('history-dialog').showModal();}
$('login-form').addEventListener('submit',async event=>{event.preventDefault();const button=event.target.querySelector('button');button.disabled=true;try{const form=new FormData(event.target);const result=await api('login',Object.fromEntries(form));csrf=result.csrf;event.target.elements.password.value='';await enter(result.wizard);}catch(error){$('login-error').textContent=error.message;}finally{button.disabled=false;}});
$('logout').addEventListener('click',guarded(async()=>{if(!confirmDiscard())return;await api('logout',{});location.reload();}));
$('file-search').addEventListener('input',renderFiles);$('category').addEventListener('change',renderFiles);$('record-search').addEventListener('input',renderRecords);
for(const [name,view]of [['form','form'],['raw','raw'],['preview','preview'],['map','map']])$(name+'-tab').addEventListener('click',guarded(()=>switchMode(view)));
$('source').addEventListener('input',mark);$('save').addEventListener('click',guarded(save));$('history').addEventListener('click',guarded(showHistory));$('close-history').addEventListener('click',()=>$('history-dialog').close());
$('validate').addEventListener('click',guarded(async()=>{if(!current)return;validInputs();const result=await api('validate',{path:current.path,text:content()});status(result.warnings?.length?'Valid; pre-existing world warnings: '+result.warnings.join('; '):'Validation passed. Ready to save.');}));
$('apply').addEventListener('click',guarded(async()=>{if(dirty)throw Error('Save or discard the current draft before applying world changes.');if(!confirm('Reload the MUD world from saved content? Connections are preserved, but active fights, NPC positions and ground items reset.'))return;$('apply').disabled=true;try{status('Validating saved areas and requesting world reload…');const result=await api('apply',{});status(result.message);}finally{$('apply').disabled=false;}}));
$('add-room').addEventListener('click',guarded(()=>{if(mode==='raw')parseSource();if(!isArea())return;const room=entryTemplate('rooms');doc.rooms.push(room);selected=doc.rooms.length-1;mode='form';mark();render();}));
$('new-file').addEventListener('click',()=>{if(confirmDiscard())$('new-dialog').showModal();});$('cancel-new').addEventListener('click',()=>$('new-dialog').close());
$('new-kind').addEventListener('change',()=>{$('new-path').value={area:'data/areas/new-area.json',help:'data/help/new-topic.md',json:'data/savage-soul/new-config.json',text:'assets/new-message.txt'}[$('new-kind').value];});
$('new-form').addEventListener('submit',event=>{event.preventDefault();const kind=$('new-kind').value;const path=$('new-path').value.trim();const value=kind==='area'?{id:id(),slug:'new-area',name:'New area',canonical:true,rooms:[]}:{};install({path,revision:'new',text:['area','json'].includes(kind)?format(value):'',activation:'New draft. Save to create the file.'});mark();$('new-dialog').close();status('New draft. Edit it, then save.');});
window.addEventListener('beforeunload',event=>{if(dirty){event.preventDefault();event.returnValue='';}});
async function ensureWorld(refresh=false){
 if(!refresh&&worldData&&Date.now()-worldAt<5000)return worldData;
 if(worldFlight)return worldFlight;
 worldFlight=api('world').then(data=>{worldData=data;worldAt=Date.now();return data;}).finally(()=>worldFlight=null);return worldFlight;
}
function mapAreas(){const areas=(worldData?.areas||[]).filter(a=>a.path!==current?.path);if(isArea())areas.push({...doc,path:current.path});return areas;}
function roomChoices(){return mapAreas().filter(area=>area.canonical===true||area.path===current?.path).flatMap(area=>(Array.isArray(area.rooms)?area.rooms:[]).map((room,index)=>({room,index,path:area.path,area:area.slug||area.name,label:(area.slug||area.name)+' / '+room.slug+' — '+room.name}))).sort((a,b)=>a.label.localeCompare(b.label));}
async function renderExitFields(holder,room){
 try{await ensureWorld();if(!holder.isConnected)return;holder.replaceChildren();const choices=roomChoices();const choose=(value,label)=>{const select=el('select');select.setAttribute('aria-label',label);const empty=el('option','Choose a room…');empty.value='';select.append(empty);if(value&&!choices.some(c=>c.room.id===value)){const missing=el('option','Missing room: '+value);missing.value=value;select.append(missing);}for(const item of choices){const option=el('option',item.label);option.value=item.room.id;select.append(option);}select.value=value||'';return select;};
 const refresh=()=>{mark();if(mode==='map')mapView.render();else renderExitFields(holder,room);};
 const exits=room.exits&&typeof room.exits==='object'?room.exits:{};
 for(const[direction,target]of Object.entries(exits)){const card=el('div',undefined,'exit-card');card.append(el('h4',direction));const select=choose(target,direction+' destination');card.append(select);select.addEventListener('change',()=>{if(!select.value)return;room.exits[direction]=select.value;refresh();});const destination=choices.find(c=>c.room.id===target),canonical=MapTopology.direction(direction),back=MapTopology.reverse[canonical];const returnExit=destination&&Object.entries(destination.room.exits||{}).find(([key])=>MapTopology.direction(key)===back);card.append(el('small',(destination?destination.label:'Missing room')+' · '+(!back?'unknown direction':!returnExit?'one-way':returnExit[1]===room.id?'return linked':'return points elsewhere')));
 const actions=el('div',undefined,'exit-actions');if(destination&&back&&destination.path===current.path){const add=btn(returnExit?'Return exists':'Add '+back+' return',()=>{if(returnExit)return;destination.room.exits??={};destination.room.exits[back]=room.id;refresh();});add.disabled=!!returnExit;actions.append(add);}else if(destination)actions.append(el('small','Cross-area return edits belong in '+destination.area+'.'));
 actions.append(btn('Remove exit',()=>{if(room.doors?.[direction]&&!confirm('Remove the '+direction+' exit and its door definition?'))return;delete room.exits[direction];if(room.doors)delete room.doors[direction];refresh();},'quiet'));card.append(actions);if(room.doors?.[direction])card.append(el('small',(room.doors[direction].hidden?'Secret ':'')+(room.doors[direction].open?'Open ':'Closed ')+(room.doors[direction].name||'door')));holder.append(card);}
 const add=el('div',undefined,'exit-add');const direction=el('select');direction.setAttribute('aria-label','New exit direction');for(const key of Object.keys(MapTopology.vectors)){if(Object.keys(exits).some(d=>MapTopology.direction(d)===key))continue;const option=el('option',key);option.value=key;direction.append(option);}const destination=choose('','New exit destination');destination.className='target-select';add.append(direction,destination,btn('Add exit',()=>{if(!direction.value||!destination.value)throw Error('Choose a direction and destination.');room.exits??={};room.exits[direction.value]=destination.value;refresh();},'add-button'));holder.append(add);
 }catch(error){holder.replaceChildren(el('p',error.message,'map-error'));}
}
mapView=new EditorMapView($('map-view'),{
 location:()=>syncLocation(),
 data:async refresh=>{const data=await ensureWorld(refresh);return{areas:mapAreas(),errors:data.errors,path:current?.path,focusKey:isArea()&&selected>=0?current.path+'#'+selected:null};},
 select:async(node,fields)=>{if(node.path!==current?.path){await openFile(node.path);if(node.path!==current?.path)return false;mapView.fitNext=true;}const index=doc.rooms.findIndex(room=>room.id===node.id);if(index<0)return false;selected=index;mode=fields?'form':'map';render();return true;},
 fields:(holder,node)=>{
  const index=node.path===current?.path&&isArea()?doc.rooms.findIndex(room=>room.id===node.id):-1;
  if(index<0){holder.append(btn('Open this area to edit the room',()=>mapView.visit(node,false)));return;}
  const room=doc.rooms[index];const actions=el('div',undefined,'map-room-actions');actions.append(btn('Save changes',save,'primary'));holder.append(actions);
  const fields=el('div',undefined,'map-room-fields');holder.append(fields);
  renderValue(fields,room,[],next=>{doc.rooms[index]=next;mark();},true);
 },
 exits:(holder,node)=>{if(node.path===current?.path&&doc?.rooms?.[node.index])renderExitFields(holder,doc.rooms[node.index]);else holder.append(btn('Open this area to edit exits',()=>openFile(node.path)));}
});
$('atlas').addEventListener('click',guarded(async()=>{if(!isArea()){const file=files.find(f=>f.path==='data/areas/savsoul.json')||files.find(f=>f.category==='World');if(!file)throw Error('No areas are available.');await openFile(file.path);if(!isArea())return;}mapView.scope='world';mapView.scopeInput.value='world';mapView.floor='all';mapView.fitNext=true;await switchMode('map');}));

bootstrap().catch(error=>{$('login-error').textContent=error.message;});
