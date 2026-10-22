NPC spawn tracking (wizard)

spawns
spawns rat

Lists managed spawn points: original home, current NPC locations, living count,
maximum population, respawn delay and next replacement countdown.
Rats respawn at home 60 seconds after death. Each home supports one living rat;
the shared rat pool supports two. Wandering rats count toward their home's cap.
A corpse does not count as a living rat. An unconscious living rat still counts.

Rat movement: once every 15 seconds, at most two rooms from home, within the
home area. They do not wander during combat or while incapacitated.
Closed and hidden doors block AI movement.

Settings: data/savage-soul/population.json, loaded at startup/copyover.
Profiles apply by NPC name; an area's NPC blueprint can supply its own spawn
block instead. Other NPCs stay unchanged unless explicitly configured.

Movement modes: stay, wander, guard_room, guard_npc.
guard_room moves to a room slug/ID, or home, and holds there.
guard_npc follows a unique NPC name or spawn ID. It waits if the target is
missing, ambiguous or unreachable. Guard objectives control positioning;
they do not initiate attacks or automatically intercept an attacker.

World reloads still reseed populations. Living positions and countdowns are
runtime state; they are not preserved across copyover.
