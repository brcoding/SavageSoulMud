//! Fixed-price merchants sharing the ordinary carried-object representation.
use bevy::ecs::message::MessageCursor;
use bevy::prelude::*;
use grim::components::{Description, Keywords, Name as GrimName, RoomDescription};
use grim::events::{Command, EngineCommand, InfoMessage};
use grim::{Actor, CarriedBy, Character, Creature, InRoom, Object, StoredCharacter, StoredObject};
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct Offer {
    pub item: StoredObject,
    pub buy_price: u64,
    pub sell_price: u64,
}

/// Stock is unlimited; each purchase creates an ordinary object instance.
#[derive(Component, Clone, Deserialize)]
pub struct Merchant {
    pub stock: Vec<Offer>,
}

pub struct ShopPlugin;
impl Plugin for ShopPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, handle_shop);
    }
}

fn reply(world: &mut World, actor: Entity, text: String) {
    world.write_message(InfoMessage {
        target: actor,
        text,
    });
}

pub(crate) fn held(world: &mut World, actor: Entity) -> Vec<(Entity, StoredObject)> {
    world
        .query_filtered::<(
            Entity,
            &GrimName,
            Option<&Description>,
            Option<&Keywords>,
            Option<&RoomDescription>,
            Option<&grim::components::ItemCondition>,
            &CarriedBy,
        ), With<Object>>()
        .iter(world)
        .filter(|(_, _, _, _, _, _, h)| h.carrier == actor)
        .map(|(e, n, d, k, r, condition, _)| {
            (
                e,
                StoredObject { condition: condition.cloned(),
                    name: n.0.clone(),
                    description: d.map(|x| x.0.clone()).unwrap_or_default(),
                    keywords: k.map(|x| x.0.clone()).unwrap_or_default(),
                    room_description: r.map(|x| x.0.clone()).unwrap_or_default(),
                },
            )
        })
        .collect()
}

/// Commit one complete character snapshot before changing live credits/items.
/// Failure leaves the transaction unapplied. The rename replaces one JSON file.
pub(crate) fn persist(world: &World, stored: &StoredCharacter) -> std::io::Result<()> {
    let dir = world
        .resource::<grim::plugins::PersistenceConfig>()
        .characters_dir();
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!("{}.json", stored.name));
    let temp = dir.join(format!("{}.shop.tmp", stored.name));
    let bytes = serde_json::to_vec_pretty(stored)?;
    use std::io::Write;
    let mut file = std::fs::File::create(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temp, dest)
}

fn handle_shop(world: &mut World, mut cursor: Local<MessageCursor<EngineCommand>>) {
    let inputs: Vec<_> = cursor
        .read(world.resource::<Messages<EngineCommand>>())
        .filter(|m| {
            matches!(
                m.command,
                Command::ListShop | Command::Buy { .. } | Command::Sell { .. } | Command::Credits
            )
        })
        .map(|m| (m.client, m.command.clone()))
        .collect();
    for (actor, command) in inputs {
        if let Err(message) = transact(world, actor, command) {
            reply(world, actor, message);
        }
    }
}

fn transact(world: &mut World, actor: Entity, command: Command) -> Result<(), String> {
    if world.get::<grim::Player>(actor).is_none() {
        return Ok(());
    }
    let character = world
        .get::<Character>(actor)
        .ok_or("You must be playing a character.")?
        .clone();
    if command == Command::Credits {
        reply(
            world,
            actor,
            format!("You have {} credits.", character.credits),
        );
        return Ok(());
    }
    let room = world
        .get::<InRoom>(actor)
        .ok_or("You are not in a room.")?
        .room;
    let mut merchants: Vec<_> = world
        .query_filtered::<(Entity, &Merchant, &InRoom, &GrimName), With<Creature>>()
        .iter(world)
        .filter(|(_, _, location, _)| location.room == room)
        .map(|(e, m, _, n)| (e, m.clone(), n.0.clone()))
        .collect();
    merchants.sort_by_key(|(e, _, _)| e.to_bits());
    let (merchant_entity, merchant, name) = merchants.first().ok_or("There is no shopkeeper here.")?;
    if world.get::<crate::savage_rules::SoulData>(*merchant_entity).is_some_and(|s|s.is_incapacitated()) {
        return Err("The shopkeeper cannot trade in their current condition.".into());
    }
    if merchant
        .stock
        .iter()
        .any(|o| o.buy_price == 0 || o.sell_price > o.buy_price)
    {
        return Err("This shop's prices need correcting.".into());
    }
    if command == Command::ListShop {
        let mut text = format!("{name}'s stock (credits):\n");
        for offer in &merchant.stock {
            text.push_str(&format!(
                "  {} — buy {}, sell {}\n",
                offer.item.name, offer.buy_price, offer.sell_price
            ));
        }
        text.push_str(&format!(
            "You have {} credits. Use buy <item> or sell <item>.",
            character.credits
        ));
        reply(world, actor, text);
        return Ok(());
    }
    let (target, buying) = match command {
        Command::Buy { target } => (target, true),
        Command::Sell { target } => (target, false),
        _ => return Ok(()),
    };
    let spec = grim::parse_target(&target, grim::ParseOptions::ITEM)
        .ok_or("Use an item name, 2.item, 2*item, or all <item>.")?;
    let pack = held(world, actor);
    let mut purchased = Vec::new();
    let mut removed = Vec::new();
    let mut total = 0u64;
    if buying {
        // Catalog row entities are temporary selector identities, never spawned.
        let catalog: Vec<_> = merchant
            .stock
            .iter()
            .enumerate()
            .map(|(i, o)| (Entity::from_raw_u32(i as u32).unwrap(), o))
            .collect();
        let matches = grim::query(
            &spec,
            catalog
                .iter()
                .map(|(e, o)| (*e, o.item.name.as_str(), o.item.keywords.as_slice())),
        );
        if matches.is_empty() {
            return Err("The shop does not stock that item.".into());
        }
        let count = match spec.selector {
            grim::Selector::Many(n) => n.get(),
            _ => 1,
        };
        // A quantity repeats the best catalog item; all buys one of each match.
        for e in matches
            .iter()
            .take(if count > 1 { 1 } else { matches.len() })
        {
            let offer = catalog.iter().find(|(id, _)| id == e).unwrap().1;
            if count > 100 {
                return Err("You can buy at most 100 items at once.".into());
            }
            let price = offer
                .buy_price
                .checked_mul(count as u64)
                .ok_or("The total is too large.")?;
            total = total.checked_add(price).ok_or("The total is too large.")?;
            purchased.extend(std::iter::repeat_n(offer.item.clone(), count));
        }
    } else {
        removed = grim::query(
            &spec,
            pack.iter()
                .map(|(e, o)| (*e, o.name.as_str(), o.keywords.as_slice())),
        );
        if removed.is_empty() {
            return Err("You are not carrying that item.".into());
        }
        for e in &removed {
            let item = &pack.iter().find(|(id, _)| id == e).unwrap().1;
            let offer = merchant
                .stock
                .iter()
                // Match the item definition, not mutable instance condition.
                .find(|o| o.item.name==item.name && o.item.description==item.description
                    && o.item.keywords==item.keywords && o.item.room_description==item.room_description)
                .ok_or_else(|| format!("{name} will not buy {}. Nothing was sold.", item.name))?;
            if offer.sell_price == 0 {
                return Err(format!(
                    "{name} will not buy {}. Nothing was sold.",
                    item.name
                ));
            }
            total = total
                .checked_add(offer.sell_price)
                .ok_or("The total is too large.")?;
        }
    }
    let balance = if buying {
        character
            .credits
            .checked_sub(total)
            .ok_or_else(|| format!("You need {total} credits; you have {}.", character.credits))?
    } else {
        character
            .credits
            .checked_add(total)
            .ok_or("Your credit balance would overflow.")?
    };
    let actor_data = world
        .get::<Actor>(actor)
        .ok_or("Character data is missing.")?;
    let actor_name = world
        .get::<GrimName>(actor)
        .ok_or("Character name is missing.")?;
    let mut stored = StoredCharacter::from_components(actor_name, actor_data, &character);
    stored.credits = balance;
    stored.inventory = pack
        .iter()
        .filter(|(e, _)| !removed.contains(e))
        .map(|(_, o)| o.clone())
        .chain(purchased.iter().cloned())
        .collect();
    stored.inventory.sort_by(|a, b| a.name.cmp(&b.name));
    persist(world, &stored).map_err(|e| {
        warn!("Shop save failed: {e}");
        "The transaction could not be saved. Your items and credits have not changed.".to_string()
    })?;
    world.get_mut::<Character>(actor).unwrap().credits = balance;
    for e in &removed {
        world.despawn(*e);
    }
    for item in &purchased {
        world.spawn((
            Object,
            GrimName(item.name.clone()),
            Description(item.description.clone()),
            Keywords(item.keywords.clone()),
            RoomDescription(item.room_description.clone()),
            item.condition.clone().unwrap_or_default(),
            CarriedBy { carrier: actor },
        ));
    }
    let count = if buying {
        purchased.len()
    } else {
        removed.len()
    };
    reply(
        world,
        actor,
        format!(
            "You {} {count} item(s) for {total} credits. You have {balance} credits remaining.",
            if buying { "buy" } else { "sell" }
        ),
    );
    Ok(())
}
