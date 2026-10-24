//! navigation implementation, split from the parent subsystem.

use super::*;

pub(super) fn edges(world: &World, room: Entity) -> Vec<(Cardinal, Entity)> {
    let mut edges: Vec<_> = world
        .get::<Exits>(room)
        .map(|e| {
            e.exits
                .iter()
                .filter(|(d, to)| {
                    world.get::<Room>(**to).is_some()
                        && !world
                            .get::<Doors>(room)
                            .and_then(|doors| doors.doors.get(d))
                            .is_some_and(|door| !door.open || door.hidden)
                })
                .map(|(d, to)| (*d, *to))
                .collect()
        })
        .unwrap_or_default();
    edges.sort_by_key(|(d, _)| *d);
    edges
}

pub(super) fn allowed_rooms(world: &World, home: Entity, movement: &Movement) -> HashSet<Entity> {
    let area = world.get::<Room>(home).map(|r| r.area);
    let mut found = HashSet::from([home]);
    let mut queue = VecDeque::from([(home, 0u16)]);
    while let Some((room, distance)) = queue.pop_front() {
        if movement.max_distance.is_some_and(|max| distance >= max) {
            continue;
        }
        for (_, to) in edges(world, room) {
            if world
                .get::<Room>(to)
                .is_some_and(|r| !movement.same_area || Some(r.area) == area)
                && found.insert(to)
            {
                queue.push_back((to, distance.saturating_add(1)));
            }
        }
    }
    found
}

pub(super) fn route(
    world: &World,
    from: Entity,
    to: Entity,
    bounds: &HashSet<Entity>,
) -> Option<Cardinal> {
    if from == to || !bounds.contains(&to) {
        return None;
    }
    let mut seen = HashSet::from([from]);
    let mut queue = VecDeque::from([(from, None)]);
    while let Some((room, first)) = queue.pop_front() {
        for (direction, next) in edges(world, room) {
            if !bounds.contains(&next) || !seen.insert(next) {
                continue;
            }
            let step = first.unwrap_or(direction);
            if next == to {
                return Some(step);
            }
            queue.push_back((next, Some(step)));
        }
    }
    None
}
