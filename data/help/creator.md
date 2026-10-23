Med-bay character creator
=========================
Visit Royal Oak Med Room and type creator. You can retrain your character,
change gender or choose a different race without losing earned XP.

creator races              List races and your XP after mandatory costs.
creator race android       Choose a race; resets unconfirmed draft training.
creator gender neutral     Choose male, female or neutral.
creator train body         Buy one training level in the draft.
creator learn quickness    Buy a supported Peculiarity in the draft.
creator buyoff no mana     Repay and remove a draft disadvantage.
creator show               Review stats, XP, credits and body parts.
creator confirm            Pay and apply this build in the med bay.
creator reset              Restart the draft from your current race.
creator cancel             Discard it without paying. Works anywhere.

The procedure costs 10,000 credits. Changing race also spends 400 XP;
changing into Android or Replicant spends 800 XP. Changing into either
synthetic race requires 6 sheets of steel, 4 steel rods and 12 rivets,
carried in your pack. Trader Sam's southfield stock sells these supplies.
Same-race retraining requires no race-change XP fee or body parts.
New character creation has its normal starting budget and no procedure fee.

Training and Peculiarity purchases are refunded into the draft. Mandatory
species traits are then budgeted at your earned XP. Confirmed race-change
XP fees stay spent permanently. You cannot confirm a negative XP balance.
Faith cannot be trained directly with XP.

Previewing or cancelling spends nothing. Confirmation rechecks the price,
XP, credits and carried parts. Only required materials are consumed. Inventory,
quests, name, account, wizard status, preferences and location are preserved.
Injuries, fatigue and spent mana are preserved; this is not a healing service.
Equipment that no longer fits is unequipped and stays in your pack.

Drafts survive reconnects. Earning/spending XP or changing your live build
makes an old draft stale: use creator reset. You cannot confirm in combat
or while incapacitated. A private backup is made before each procedure.

For wizards: edit data/savage-soul/creator.json under Rules in the wizard
editor. Set procedure_credits, race_change_xp, race_change_xp_overrides,
race_change_materials (crafting item IDs and positive quantities), and
med_bays (stable area/room slugs). Changes apply on the next command.
Existing drafts must be reset if their quoted costs change.
