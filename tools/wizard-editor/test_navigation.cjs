const assert=require('node:assert/strict');
const test=require('node:test');
require('./static/map-view.js');

function view(select){
 const node={id:'room'};
 return Object.assign(Object.create(globalThis.EditorMapView.prototype),{
  scope:'world',floor:'all',fitNext:false,scopeInput:{value:'world'},
  model:{byKey:new Map(),nodes:[node]},hooks:{select},renders:0,
  async render(){this.renders++;}
 });
}

test('declining region navigation preserves atlas state',async()=>{
 const v=view(async()=>false);
 assert.equal(await v.visit({},false,{scope:'area',floor:'auto',fitNext:true}),false);
 assert.equal(v.scope,'world');assert.equal(v.floor,'all');
 assert.equal(v.scopeInput.value,'world');assert.equal(v.fitNext,false);
 assert.equal(v.renders,1);
});

test('town and room presets preserve atlas after cancel',async()=>{
 for(const preset of ['town','room']){
  const v=view(async()=>false);
  assert.equal(await v.preset(preset),false);
  assert.equal(v.scope,'world');assert.equal(v.floor,'all');
 }
});

test('failed region navigation restores state and propagates the error',async()=>{
 const failure=new Error('Unable to open area');
 const v=view(async()=>{throw failure;});
 await assert.rejects(v.visit({},false,{scope:'area',floor:'auto'}),failure);
 assert.equal(v.scope,'world');assert.equal(v.floor,'all');
});

test('successful region navigation keeps the new state',async()=>{
 const v=view(async()=>true);
 assert.equal(await v.visit({},false,{scope:'area',floor:'auto',fitNext:true}),true);
 assert.equal(v.scope,'area');assert.equal(v.floor,'auto');assert.equal(v.renders,0);
});

test('an older canceled visit cannot roll back a newer successful visit',async()=>{
 let finishOld;
 const oldResult=new Promise(resolve=>{finishOld=resolve;});
 const v=view(node=>node.id==='old'?oldResult:Promise.resolve(true));
 const older=v.visit({id:'old'},false,{scope:'area',floor:'auto',fitNext:true});
 assert.equal(await v.visit({id:'new'},false,{scope:'nearby',floor:'2',fitNext:false}),true);
 finishOld(false);await older;
 assert.equal(v.scope,'nearby');assert.equal(v.floor,'2');
 assert.equal(v.scopeInput.value,'nearby');assert.equal(v.fitNext,false);
 assert.equal(v.renders,0);
});

test('an older failed visit cannot roll back a newer successful visit',async()=>{
 let failOld;
 const oldResult=new Promise((_,reject)=>{failOld=reject;});
 const v=view(node=>node.id==='old'?oldResult:Promise.resolve(true));
 const older=v.visit({id:'old'},false,{scope:'area',floor:'auto'});
 await v.visit({id:'new'},false,{scope:'nearby',floor:'3'});
 failOld(new Error('old request failed'));await assert.rejects(older,/old request failed/);
 assert.equal(v.scope,'nearby');assert.equal(v.floor,'3');assert.equal(v.renders,0);
});

test('canceling overlapping visits restores the last committed atlas state',async()=>{
 let finishOld;
 const oldResult=new Promise(resolve=>{finishOld=resolve;});
 const v=view(node=>node.id==='old'?oldResult:Promise.resolve(false));
 const older=v.visit({id:'old'},false,{scope:'area',floor:'auto',fitNext:true});
 assert.equal(await v.visit({id:'new'},false,{scope:'nearby',floor:'2'}),false);
 finishOld(false);await older;
 assert.equal(v.scope,'world');assert.equal(v.floor,'all');
 assert.equal(v.scopeInput.value,'world');assert.equal(v.fitNext,false);
});

test('a subsequent canceled visit restores the newer committed selection',async()=>{
 const v=view(async()=>true);
 await v.visit({},false,{scope:'area',floor:'1'});
 v.hooks.select=async()=>false;
 await v.visit({},false,{scope:'nearby',floor:'2'});
 assert.equal(v.scope,'area');assert.equal(v.floor,'1');
});
