# NPC populations and objectives

The live rat profile in `data/savage-soul/population.json` uses:

```json
{
  "profiles": {
    "rat": {
      "max_alive": 1,
      "respawn_seconds": 60,
      "pool": "savsoul-rats",
      "max_total": 2,
      "movement": {
        "mode": "wander",
        "interval_seconds": 15,
        "max_distance": 2,
        "same_area": true
      }
    }
  }
}
```

Each original blueprint placement is a spawn point. Its stable fallback identity
is area GrimId + room GrimId + NPC array index. An optional `id` in a spawn block
provides an explicit identity for references. Runtime members retain their home
even when wandering. All living members count toward the point and optional
shared pool budget regardless of room. Death/removal releases one slot and starts
its delay; corpse decay does not affect population accounting. Pool cooldowns
prevent another vacant point from replacing a killed rat ahead of that delay.
Incapped but living creatures still occupy a slot. Replacement uses the entire
original blueprint, including soul profile, combat behavior, scripts and shop.

No spawn settings means no managed respawn or AI. This preserves all existing
NPCs and the bespoke Archwraith quest timer. Profiles are keyed by lowercase NPC
name. A blueprint `spawn` block replaces the profile completely; omitted fields
use defaults (one NPC, 60-second respawn, stationary movement). Zero max_alive or
max_total spawns no NPCs for that point or pool. Omitting max_total removes the
pool limit. Limits are clamped to 100 per point, 1000 per pool,
respawn 1..86400 seconds and movement 1..3600 seconds. Inconsistent pool maximums
use the lowest declared value. Config changes require restart/copyover.

Movement examples inside a spawn block:

```json
{"movement":{"mode":"guard_room","target":"romainstwallgate","same_area":true}}
```

```json
{"movement":{"mode":"guard_npc","target":"Sue Rodgers","max_distance":3,"same_area":true}}
```

`target` for guard_room accepts a unique room slug, room GrimId, or `home`.
guard_npc accepts a unique NPC name or an explicit spawn ID. Ambiguous, absent or
unreachable targets wait; no teleportation. Guard modes govern positioning only,
not automatic hostility/protection. `max_distance` is optional; when absent there
is no radius limit. `same_area` defaults true and can independently be false.
All steps go through the ordinary Move command and transition vetoes, including
gate checks and player-visible movement. Closed/hidden doors are unavailable to
AI. NPCs pause movement while fighting or incapacitated, with a final AttemptWalk
guard to stop stale queued moves after combat begins.

Use `spawns` or `spawns rat` as a wizard for live counts, homes, locations and
countdowns. Registration, vacancies and replacement events are also logged.
Population timers and wandering locations reset when the world reloads; this
does not add persistent world-NPC storage. Ground corpses follow existing reset
semantics. Tests of command interactions disable the dynamic controller to retain
their fixed encounter fixtures; separate manual-clock tests cover real caps,
cooldowns, moved membership, respawn profiles, radius/area bounds and objectives.
