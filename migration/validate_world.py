"""Validate every converted room is reachable, returnable, and linked correctly."""
import json, pathlib, re
root = pathlib.Path(__file__).resolve().parents[1]
areas = [json.loads(p.read_text(encoding='utf-8-sig')) for p in (root/'data'/'areas').glob('*.json')]
rooms = {r['id']:r for a in areas for r in a['rooms']}
assert len(rooms) == sum(len(a['rooms']) for a in areas), 'Duplicate IDs'
directions = {'north','east','south','west','up','down'}
for a in areas:
    assert re.fullmatch('[A-Za-z0-9]{12}', a['id'])
    for r in a['rooms']:
        assert re.fullmatch('[A-Za-z0-9]{12}', r['id'])
        assert set(r['exits']) <= directions
        assert all(t in rooms for t in r['exits'].values()), r['slug']
        assert set(r.get('doors',{})) <= set(r['exits']), r['slug']
start = next(a['starting_room'] for a in areas if a.get('starting_room'))
def reachable(origin):
    seen, todo = set(), [origin]
    while todo:
        current = todo.pop()
        if current in seen: continue
        seen.add(current)
        todo.extend(rooms[current]['exits'].values())
    return seen
converted = next(a for a in areas if a['slug']=='savsoul')['rooms']
assert len(converted)==20
assert {r['id'] for r in converted} <= reachable(start)
assert all(start in reachable(r['id']) for r in converted)
tavern = next(r for r in rooms.values() if r['slug']=='tavern')
gate = next(r for r in converted if r['slug']=='romainstwallgate')
assert tavern['exits']['west']==gate['id'] and gate['exits']['east']==tavern['id']
assert tavern['doors']['west']==gate['doors']['east']
assert not gate['doors']['east']['open']
print(f'PASS: {len(rooms)} rooms, 20 converted rooms reachable and returnable, reciprocal closed Savsoul door.')
