# Production backup identified — 2026-10-15

Authoritative recovered source: P:\savsoulmud\dev37prodback\ds3.7a7\lib. This supersedes the older mudbackup and ds3.6 snapshots for future recovery. Source files remain untouched.

The eight Bar N Grill rooms now use their source SetShort/SetLong, SetItems and exits. This corrects our invented dining room (actually an empty booze storeroom), western bar (actually stove/cooking room), and western bedroom (twin-bed rental). Rusty's appearance and aliases are source-derived. The north bedroom contains the source guest bed and trapped desk descriptions. Digital forum presence is retained. Reproduce with migration/restore_bar_from_backup.py; supporting source is archived in migration/production-bar-source.

Quest evidence recovered:
- barngrilldesk.c: closed, immovable storage; level-1 poison needle on opening, no auto reset; contains mist dagger and dagger note.
- daggernote.txt: wield the mist dagger and attack the wraith.
- wraith.c: special response to wielded mist dagger; consumes dagger and awards Savior of Royal Oak.
- ROMainStWallGate.c: south exit callback requires Escape from Royal Oak OR Savior of Royal Oak, plus a carried Soulboy. Quest gate does not simply check a physical key.
- gateterminal.c and Walt/Sue dialogue contain the alternative route and need a full script port.

Static conversion is deployed. This update does NOT implement storage/container opening, disarm/poison, contained-item retrieval, readable note interaction, Rusty's conversation/vendor/crafting, forum posts, wraith special combat or quest gate checks. These source scripts are now available for porting; claiming this quest is playable would be incorrect. Other domains and unvisited Savsoul rooms still reflect the previous conversion until reviewed against this production backup.

Update: the Bar N Grill desk interaction is now ported: deterministic disarm, closed/open state, one-use needle, saved dagger/note extraction, readable note and wieldable dagger. Desk state is per character. Venom is a game adaptation at Body DF10, with rulebook five-second checks and cumulative wounds. Original forum/vendor/wraith/gate scripting still pending.

Update: all seventeen marsh rooms, production Faded Trail links, Archwraith, Nurse Sue, Soulboy provisioning and the mist-dagger quest are ported. The complete gameplay route through the south gate passes the end-to-end check. Source reward 2000 XP and 10 quest points is preserved once per character. Atomic save includes dagger removal, key, Soul state and quest data. Other players can use the wraith after its sixty-second return; source random placement is made reliable. Ordinary attacks are refused rather than emulating the old permanent-stat-drain script. Alternative terminal override remains pending; Morlock/wasteland boundaries remain as previously reported.

Ordinary imported rats now have explicit small-animal stats and authored retreat behavior instead of default human combat stats. Their body/leadership/attack/defense/Will Power choices are game balancing, not a direct conversion of LPC HP. Player combat grammar is corrected.

Correction: removed the rat retreat workaround. Small-target Trample now provides a rule-based playable attack against ordinary rats, with Desperation Dive avoidance. Kill can finish unconscious opponents instead of stopping short of death.
