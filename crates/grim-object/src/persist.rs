//! Pack persistence: snapshot carried objects into the character file and
//! re-spawn them on login.
//!
//! Every carried instance is stored whole (never by reference). Ground objects
//! are never stored — they regenerate from area blueprints — so after a
//! pickup-then-reboot the world holds two of the seed item (one regrown on the
//! ground, one restored into the pack). That duplication is correct state, and
//! dropping the carried copy then leaves two on the ground, equally correct.
//! A further reboot wipes ground extras and regrows the single blueprint copy.

use bevy::prelude::*;
use grim_actor::StoredObject;
use grim_core::components::{Description, Keywords, Name as GrimName, RoomDescription};

use crate::object::{CarriedBy, Object};
use grim_core::components::ItemCondition;

/// One carried object's snapshot inputs, as read from the world.
pub type Held<'a> = (
    &'a GrimName,
    Option<&'a Description>,
    Option<&'a Keywords>,
    Option<&'a RoomDescription>,
    Option<&'a ItemCondition>,
    &'a CarriedBy,
);

/// Snapshot every object in `held` belonging to `carrier`, sorted by short
/// name so the file is deterministic. Full instance data: name, look
/// paragraphs, keywords, room line.
pub fn snapshot<'a>(held: impl Iterator<Item = Held<'a>>, carrier: Entity) -> Vec<StoredObject> {
    let mut out: Vec<(String, StoredObject)> = held
        .filter(|(_, _, _, _, _, held)| held.carrier == carrier)
        .map(|(nm, desc, kw, line, condition, _)| {
            (
                nm.0.clone(),
                StoredObject { condition: condition.cloned(),
                    name: nm.0.clone(),
                    description: desc.map(|d| d.0.clone()).unwrap_or_default(),
                    keywords: kw.map(|k| k.0.clone()).unwrap_or_default(),
                    room_description: line.map(|l| l.0.clone()).unwrap_or_default(),
                },
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter().map(|(_, stored)| stored).collect()
}

/// Snapshot `carrier`'s pack straight from the system query.
pub fn snapshot_pack(pack: &Carried, carrier: Entity) -> Vec<StoredObject> {
    snapshot(
        pack.iter()
            .map(|(_, nm, desc, kw, line, condition, held)| (nm, desc, kw, line, condition, held)),
        carrier,
    )
}

/// Carried objects as a system query: the marker, descriptive components, and
/// carrier link. Ground objects (no `CarriedBy`) never match.
pub type Carried<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static GrimName,
        Option<&'static Description>,
        Option<&'static Keywords>,
        Option<&'static RoomDescription>,
        Option<&'static ItemCondition>,
        &'static CarriedBy,
    ),
    With<Object>,
>;

/// The live bundle for one snapshot entry, carried by `carrier`.
pub fn restore_bundle(
    stored: &StoredObject,
    carrier: Entity,
) -> (
    Object,
    GrimName,
    Description,
    Keywords,
    RoomDescription,
    CarriedBy,
    ItemCondition,
) {
    (
        Object,
        GrimName(stored.name.clone()),
        Description(stored.description.clone()),
        Keywords(stored.keywords.clone()),
        RoomDescription(stored.room_description.clone()),
        CarriedBy { carrier },
        stored.condition.clone().unwrap_or_default(),
    )
}

/// Re-spawn a login snapshot into `carrier`'s pack. Ground listings are
/// untouched: a seed twin on the ground stays where it is.
pub fn restore(commands: &mut Commands, carrier: Entity, inventory: &[StoredObject]) {
    for stored in inventory {
        commands.spawn(restore_bundle(stored, carrier));
    }
}

/// Despawn `entities` (quit unload: the snapshot already went to disk, so the
/// live entities must go or their `CarriedBy` dangles at the dead character).
/// The caller collects what its carrier holds.
pub fn unload(commands: &mut Commands, entities: impl Iterator<Item = Entity>) {
    for entity in entities {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Owned snapshot inputs for tests (`snapshot` takes references).
    type OwnedHeld = (
        GrimName,
        Option<Description>,
        Option<Keywords>,
        Option<RoomDescription>,
        Option<ItemCondition>,
        CarriedBy,
    );

    fn held(world: &mut World) -> Vec<OwnedHeld> {
        // Owned copies: snapshot takes references, so materialize through the
        // world's query state first.
        let mut state = world.query::<(
            &GrimName,
            Option<&Description>,
            Option<&Keywords>,
            Option<&RoomDescription>,
            Option<&ItemCondition>,
            &CarriedBy,
        )>();
        state
            .iter(world)
            .map(|(nm, d, k, l, c, h)| (nm.clone(), d.cloned(), k.cloned(), l.cloned(), c.cloned(), *h))
            .collect()
    }

    #[test]
    fn snapshot_holds_only_the_carriers_sorted() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let alice = app.world_mut().spawn_empty().id();
        let bob = app.world_mut().spawn_empty().id();
        for (name, carrier) in [("sword", alice), ("coin", alice), ("shield", bob)] {
            app.world_mut().spawn((
                Object,
                GrimName(name.into()),
                Description(vec![format!("{name} desc")]),
                Keywords(vec![format!("{name} kw")]),
                RoomDescription(format!("{name} line")),
                CarriedBy { carrier },
            ));
        }

        let owned = held(app.world_mut());
        let snap = snapshot(
            owned
                .iter()
                .map(|(nm, d, k, l, c, h)| (nm, d.as_ref(), k.as_ref(), l.as_ref(), c.as_ref(), h)),
            alice,
        );
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].name, "coin");
        assert_eq!(snap[1].name, "sword");
        assert_eq!(snap[0].description, vec!["coin desc".to_string()]);
        assert_eq!(snap[0].keywords, vec!["coin kw".to_string()]);
        assert_eq!(snap[0].room_description, "coin line");
    }

    #[test]
    fn restore_bundle_rebuilds_the_live_parts() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let carrier = app.world_mut().spawn_empty().id();
        let stored = StoredObject { condition: Some(ItemCondition {quality:45,decay:Some(grim_core::components::ItemDecay{started_at:100,expires_at:400})}),
            name: "brass lantern".into(),
            description: vec!["A sturdy brass lantern.".into()],
            keywords: vec!["lantern".into()],
            room_description: "A brass lantern rests here.".into(),
        };
        let (obj, nm, desc, kw, line, held, condition) = restore_bundle(&stored, carrier);
        assert_eq!(Some(condition),stored.condition);
        let _ = obj;
        assert_eq!(nm.0, "brass lantern");
        assert_eq!(desc.0, vec!["A sturdy brass lantern.".to_string()]);
        assert_eq!(kw.0, vec!["lantern".to_string()]);
        assert_eq!(line.0, "A brass lantern rests here.");
        assert_eq!(held.carrier, carrier);
    }
}
