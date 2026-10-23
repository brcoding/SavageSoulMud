Races and species templates
==========================

races                 List all choices and unspent starting XP.
race                  Inspect your saved template, traits and costs.
race <slug>           Preview a template (for example: race android).
buyoff <disadvantage> Repay its capacity refund and remove its restriction.

Choose a race after gender during character creation. All characters begin with
600 total XP and 3 earned C. Mandatory species traits use that capacity first;
extra C costs 100 XP each. One C can be converted into two NC. Disadvantages
refund C/NC, not XP. Cluster-included disadvantages do not refund points or
consume extra disadvantage slots. Costs are shown before choosing a race.

Default templates (MUD-authored combinations, not official species stat blocks):
- Human/Zister: 600 XP unspent, 3 C free; no mandatory traits.
- Human-Sheltie: 600 XP, 6 C free; No Mana and No Mutations grant 3 extra C.
  Mana remains zero; mutation traits such as Quickness cannot be learned.
- Android: Electronic package; 200 XP unspent, no free C.
- Gurnin: Brutish and Natural Armor 2; 600 XP, no free C.
  +1 Strength, -1 Intellect, light leathery hide.
- Tater: Stature Large 2 and Burly; 500 XP, no free C.
  Size/Mass 7, Body 8 before training; Athletics/Defense 4, Dodge 2 and
  Passive Defense 8. Greater strength and carrying capacity, easier to hit.
  Standard Size-5 armor does not fit; larger weapons can be used.
- Morlock: Brutish, Sharp Strike 2, Enhanced Vision 1, Carnivore 1;
  500 XP, no free C. Natural claws and night vision; meat-only diet.
- Replicant: Electronic and Deceptive Appearance 1; 0 XP, no free C, 1 NC free.
  Human-like synthetic exterior. Earn XP through play before buying training.
- Poleepkwa: Brutish, Natural Armor 2 and Sharp Strike 1;
  500 XP, no free C. Light chitin armor and cutting claws.

Only one Attribute Modifier (Brainy, Brutish, Burly, Genteel) is allowed,
including one already supplied by your species. Natural and worn armor never
stack; the stronger layer applies, then its coverage can be defeated. Claws
are used automatically when unarmed and at least as effective as a blunt hit;
they do not increase the damage of a wielded sword or a trample.

Electronic beings
-----------------
Electronic costs 5C+4NC. It includes Hard to Kill, Heal/Harm as Object,
No Strings Attached, Chemically Disadvantaged 2, Resistant to Aging,
Cold 2, Disease 2, Heat 2, Poison 2, Pressure, Radiation 2,
Starvation and Dehydration 2, Vacuum, Respiration 3,
Unusual Substance (Inorganic), and Non-Mutant Augmentation.

Hard to Kill removes the natural -2 Sharp DR penalty, but reduces armor's Sharp
DR bonus by 2. It prevents death from critical-injury deterioration and
stabilizes after 16 hours. Direct attacks can still kill. No free immunity
to fatigue, winded, stun or fear is included.

Electronic characters resist venom. Ordinary medical healing and beneficial
chemicals/potions cannot heal them. They need repairs. Royal Oak's medical
table has an authored mechanical repair mode for these patients. BTP/Harm
immunity and EMP/Break vulnerability are part of the recorded package;
no immunity to EMP is granted. Electronic itself does not remove mana.
No Mutations cannot be added to an Electronic/non-mutant-augmentation build.

Buyoff uses unused capacity first and purchases any missing C for 100 XP each.
For an NC disadvantage it may also convert C to NC. It never grants an XP
refund. Buying off No Mana makes a mana pool possible; it does not refill it.
Paid species packages cannot exchange their spent build capacity for credits.

Night vision and light
----------------------
Night vision ignores room darkness, not smoke, fog, invisibility or walls.
An authored room with dark=true hides its description, contents and map unless
a viewer has night vision or a light illuminates the room. No vision imposes
-3 on visual Perception, melee/ranged attack and active Defense.
A carried or placed object with keyword light:active illuminates its room.
Lighting is explicit room metadata; existing prose does not automatically
make a room dark. Items in your own pack remain inspectable by touch.
Carnivores can eat meals marked diet=meat or food objects tagged food:meat;
generic rations and mixed meals are refused without consuming them.

Current subsystem limits
------------------------
The wider spellcasting, aging, oxygen, disease, weather and social-prejudice
systems are not implemented. Corresponding traits are recorded for those
systems; they do not create extra stat bonuses or immunity to unrelated harm.

Editing and provenance
----------------------
Edit data/savage-soul/races.json under Rules in the wizard editor. The MUD
validator checks supported trait levels, conflicts and starting affordability
before saving. Reload/copyover to offer the new templates. Existing characters
retain their saved template snapshot: edits never silently change their traits,
costs, purchases or race. Changing an existing character's build requires a
med-bay procedure. Use help creator for retraining, race-change costs and
synthetic body parts. Earned XP is preserved and costs are previewed.

Book references (2026 printed pages): A1 disadvantages; A2 modifiers (272),
Electronic (274), Enhanced Vision (283), Deceptive Appearance (309),
Hard to Kill (316), Heal/Harm as Object (317), Natural Armor (324),
Sharp Strike (331), Stature Large (334), Non-Mutant Augmentation (354),
Species (355). Rulebook Faith policy remains: no direct XP training.
