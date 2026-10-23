"""Recover authored development crafting definitions; never executes LPC or changes source.

Run from any directory with --source LIB --equipment-base LIVE/items.json.
Only writes this checkout's catalog and a merge artifact; room merging is separate.
"""
from pathlib import Path
import argparse, copy, hashlib, json, re

ROOT = Path(__file__).resolve().parents[1]
WORK = ROOT.parents[1] / 'work'
helpers = {'__file__': str(ROOT / 'migration/convert_population.py')}
exec((ROOT / 'migration/convert_population.py').read_text(encoding='utf-8').split('sources={}')[0], helpers)
calls, strings, clean, uncomment = (helpers[k] for k in ('calls','strings','clean','uncomment'))

def value(text, name, default=''):
    found = calls(text,name)
    return clean(''.join(strings(found[-1]))) if found else default

def slug(text):
    return re.sub(r'[^a-z0-9]+','-',text.lower()).strip('-')

def mapping(text,name):
    found=calls(text,name)
    return {k:int(v) for k,v in re.findall(r'"([^"\n]+)"\s*:\s*(\d+)',found[-1])} if found else {}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True)
    parser.add_argument('--equipment-base',type=Path,required=True)
    args=parser.parse_args(); lib=args.source
    catalog={'items':{},'recipes':[],'reductions':[],'meals':{},'source_items':{}}
    items=catalog['items']; source_ids={}; key_ids={}; prices={}; fixtures={}
    equipment=json.loads(args.equipment_base.read_text(encoding='utf-8-sig'))
    gear_base={x['name']:x for x in equipment}
    paths=[]
    for sub in ('tools','resources','meals','weap','armor'):
        paths+=sorted((lib/f'domains/crafting/{sub}').rglob('*.c'))
    paths += [lib/'domains/savsoul/weap/GH3round.c',lib/'domains/savsoul/weap/GH3shell.c',lib/'domains/savsoul/meals/ham_sand.c',lib/'domains/savsoul/meals/pbnj_sand.c']
    # Legacy reduction input templates; only actual source files are imported.
    for basename in ('short_sword.c','wrench.c'):
        candidates=list((lib/'domains').rglob(basename))
        if candidates: paths.append(candidates[0])
    for path in paths:
        raw=path.read_text(encoding='utf-8',errors='replace'); text=uncomment(raw)
        key=value(text,'SetKeyName'); short=value(text,'SetShort')
        if not key or not short: continue
        rel=path.relative_to(lib).as_posix(); ident=slug(path.stem)
        if path.stem=='GH3shell': ident='gh3-shell'
        if path.stem=='GH3round': ident='gh3-round'
        name=re.sub(r'^(?:a|an|the)\s+','',short,flags=re.I)
        quality=re.search(r'_q([0-3])$',path.stem)
        if quality: name+=f' (quality {quality[1]})'
        description=value(text,'SetLong',short)
        if quality: description+=f' Craftsmanship tier: {quality[1]} of 3. Wear is tracked separately.'
        aliases=[key,name,ident.replace('-',' ')]+sum([strings(x) for x in calls(text,'SetId')+calls(text,'SetAdjectives')],[])
        keywords=list(dict.fromkeys([x.lower() for x in aliases if x]+['craft:'+ident]))
        if 'knife' in key.lower() and ('/weap/' in rel): keywords+=['craft:knife']
        item={'condition':{'quality':100},'name':name,'description':[description], 'keywords':keywords,'room_description':short[0].upper()+short[1:]+' is here.'}
        items[ident]=item; source_ids['/'+rel.removesuffix('.c')]=ident
        key_ids.setdefault(key.lower(),ident)
        # Source resource Value and tool/equipment BaseCost were already credits.
        found=calls(text,'SetValue'); amount=re.search(r'\d+',found[-1]) if found else None
        if not amount:
            found=calls(text,'SetBaseCost'); amount=re.search(r',\s*(\d+)',found[-1]) if found else None
        price=int(amount[1] if amount and amount.lastindex else amount[0]) if amount else 10
        prices[ident]=max(1,price)
        catalog['source_items'][ident]={'source':rel,'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'price_credits':prices[ident]}
        if quality: catalog['source_items'][ident]['craftsmanship']=int(quality[1])
        if 'inherit LIB_TABLE' in text:
            fixtures[ident]=item
        elif ('/weap/' in rel or '/armor/' in rel or ('/tools/' in rel and 'SetWeaponType' in text)) and not any(x in text for x in ('inherit LIB_ROUND', 'inherit LIB_SHELL')):
            if '/armor/' in rel: base='Medium Armor'
            elif 'knife' in key.lower() or 'chisel' in key.lower(): base='Pocket Knife' if 'small_knife' in rel else 'Combat Knife'
            elif value(text,'SetWeaponType') in ('blade','pierce'):
                base='Heavy Sword' if any(x in path.stem for x in ('huge_','large_','halberd')) else 'Light Sword'
            else: base='Heavy Club' if any(x in path.stem for x in ('huge_','large_','medium_')) else 'Light Club'
            entry=copy.deepcopy(gear_base[base]);entry.update(name=name,keywords=keywords,price=prices[ident],sell_price=prices[ident]//2)
            equipment=[x for x in equipment if x['name']!=name]+[entry]
            catalog['source_items'][ident]['equipment_adapter']=base
        if 'inherit LIB_MEAL' in text:
            messages=calls(text,'SetMealMessages')
            message=strings(messages[-1])[0] if messages else f'You eat {name}.'
            catalog['meals'][ident]={'verb':'eat','message':clean(message)}
    # Explicit aliases disambiguate repeated legacy key names (wood, meat, sandwich).
    for key,ident in {'wood handle':'wood-handle','piece of wood':'piece-of-wood','gh3shell':'gh3-shell','gh3 round':'gh3-round','ham sandwich':'ham-sand','pliers':'pliers','short sword':'short-sword','wrench':'wrench'}.items():
        key_ids[key]=ident
    def ingredient_id(key): return key_ids.get(key.lower(),slug(key))
    for category,method,ingredient_method in [('recipes','SetRecipeName','SetIngredients'),('reductions','SetReduceName','SetRequiredItem')]:
        for path in sorted((lib/f'crafting/{category}').glob('*.c')):
            text=uncomment(path.read_text(encoding='utf-8',errors='replace'))
            if not calls(text,method): continue
            name=value(text,method)
            if not name:
                match=re.search(r'RECIPE_NAME\s*=\s*"([^"]+)"',text); name=match[1] if match else path.stem.replace('_',' ')
            aliases=[name,slug(name).replace('-',' ')]
            if name=='plier': aliases+=['pliers']
            if name=='peanut butter and jelly sandwich': aliases+=['pbj','pbnj','pbj sandwich']
            if name.endswith(' corpse'):
                creature=name.removesuffix(' corpse');aliases += ['corpse of the '+creature,'corpse of '+creature]
            products={source_ids[k.removesuffix('.c')]:v for k,v in mapping(text,'SetProduct').items()}
            ingredients={ingredient_id(k):v for k,v in mapping(text,ingredient_method).items()}
            tools=[slug(x) for x in strings(calls(text,'SetTool')[-1])] if calls(text,'SetTool') else []
            entry={'id':slug(name),'name':name,'aliases':list(dict.fromkeys(aliases)),'ingredients':ingredients,'tools':tools,'products':products,'unavailable':None,'source':path.relative_to(lib).as_posix(),'legacy_skills':mapping(text,'SetSkill')}
            if 'bandsaw' in tools: entry['unavailable']='The development backup contains no bandsaw tool. Buy a wood handle from Metalhead instead.'
            catalog[category].append(entry)
    def offer(ident): return {'item':items[ident],'buy_price':prices[ident],'sell_price':prices[ident]//2}
    def storage_items(rel):
        text=uncomment((lib/rel).read_text(errors='replace'))
        return [source_ids[x.removesuffix('.c')] for x in strings(''.join(calls(text,'SetInventory'))) if x.removesuffix('.c') in source_ids]
    merchants={
        'metalhead':storage_items('domains/southfield/room/MetalheadStorage.c')+['wood-handle','pliers','chisel','claw-hammer','piece-of-wood'],
        'rusty':storage_items('domains/savsoul/room/rustystorage.c')+['knife-q0'],
        'ron metalswiller':storage_items('domains/savsoul/room/ronmetalswillerstorage.c'),
        'trader sam':storage_items('domains/savsoul/room/ROTraderSamsStorage.c'),
    }
    patches={'merchants':{name:[offer(i) for i in dict.fromkeys(stock)] for name,stock in merchants.items()},'roomfixturepatches':{'robarngrill03':[fixtures['stove']],'southfieldmetalshop':[fixtures[k] for k in ('bullet-press','grinding-wheel','die-forge','radial-rivet-machine')]},'source_ids':source_ids}
    def write(path,obj): path.parent.mkdir(parents=True,exist_ok=True);path.write_text(json.dumps(obj,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
    write(ROOT/'data/savage-soul/crafting.json',catalog)
    write(ROOT/'data/savage-soul/items.json',equipment)
    write(WORK/'dev-crafting-merchants.json',patches)
    print(json.dumps({'items':len(items),'recipes':len(catalog['recipes']),'reductions':len(catalog['reductions']),'meals':len(catalog['meals']),'merchant_offers':{k:len(v) for k,v in patches['merchants'].items()},'fixture_count':len(fixtures)},indent=2))

if __name__=='__main__':main()
