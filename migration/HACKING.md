# Configurable terminals

The game-local `src/hack.rs` handler uses ordinary scenery fixtures. Add
`hackable` to an area's scenery entry (not to pickable objects):

```json
{"id":"unique-device-id","attempts":4,"lockout_seconds":60,
 "effect":{"kind":"open_exit","area":"savsoul","room":"room-slug","direction":"south"}}
```

IDs must be unique within their room and stable across updates. Attempts
must be 4–20. The curated eight-word bank is solvable within four guesses.
Other effects are `{"kind":"access"}` (saved access only) and
`{"kind":"royal_oak_gate"}` (gate override and recovered quest reward).
The latter preserves the canonical Escape from Royal Oak reward of 2000 XP
and 10 quest points once; the existing Soulboy exit guard remains in force.
It replaces the recovered hardcoded-login interaction with the requested
deduction game. It does not introduce a Savage Soul skill roll.

Open-exit effects validate the room, exit and door before accepting guesses.
They open the reverse door when a reciprocal exit/door exists. Effects are
idempotent: use `hack terminal` on a solved terminal after a world restart
or another player closing its door. No repeat rewards are issued.

State lives in the character's `game_state.hacking`, keyed by stable room
GrimId plus device ID: candidate order, secret, feedback history, remaining
attempts, completion and absolute lockout deadline. It is saved atomically
with the whole pack, quest record and SoulData before live effects. Abort,
movement and reconnect preserve progress. Guesses require the bound device
in the current room and an able character outside combat. Bad state or
configuration fails closed. No timer process or modal input state is needed.

Edit `data/help/hack.md` for player instructions. The Royal Oak fixture also
lives in `migration/savsoul-reconstruction.json` so re-import keeps it.
