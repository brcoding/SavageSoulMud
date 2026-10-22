# Player display preferences

The local server asks once per character, after authentication and MOTD, whether
they use a screen reader. Answer yes or no; Enter defaults to no. Existing logged
in sessions continue through copyover and can use `screenreader on` immediately.

Commands save immediately, including the complete inventory:

```
screenreader on
screenreader off
combatlog brief
combatlog normal
combatlog detailed
prompt compact
prompt status
prompt combat
prompt accessible
prompt classic
prompt off
prompt custom {name}: {condition} | {xp} XP | {credits} cr>
```

`screenreader on` selects plain text, no ASCII maps, brief combat and `Ready.`
as the prompt. Turning it off restores earlier display settings. `prompt off`
also works in this mode. `map` becomes an exit list. The initial greeting is
plain text so the first login does not begin with an ASCII logo.

Normal combat is short, with no repeated initiative/roll narration. Brief gives
one line per round with hits, misses, wounds, recovery and the outcome;
detailed retains rule calculations. Messages
for a round share one output and the transport sends one prompt for a frame's
output. Example normal output:

```
You trample rat: injured, stunned.
[OK] >
Rat killed. XP +5.
[OK] >
```

NPC rewards are authored through `combat_behavior.xp_reward`. Rats pay 5 XP once
per spawned creature's defeat. Finishing an already rewarded incapacitated NPC
does not pay twice. XP commits with full character state before being announced.
Other NPCs default to zero until assigned rewards. Every ended fight reports XP,
including zero for stopping, losing reach or fleeing. The wraith's separate quest
reward remains 2000 XP; brief mode gives a compact completion message.

Custom tokens: `{name}`, `{condition}`, `{xp}` (total), `{credits}`, `{mana}`,
`{maxmana}`, `{opponent}`. Templates allow 1-160 printable plain-text characters.
Help topics `screenreader` (alias `accessibility`), `combatlog`, `prompt` and
`config` live in `data/help`; they can be edited without rebuilding.
