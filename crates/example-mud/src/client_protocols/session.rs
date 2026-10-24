//! session implementation, split from the parent subsystem.

use super::*;

pub(super) fn tick(world: &mut World, mut cursor: Local<MessageCursor<TelnetInput>>) {
    let incoming: Vec<_> = cursor
        .read(world.resource::<Messages<TelnetInput>>())
        .cloned()
        .collect();
    let mut state = world.remove_resource::<State>().unwrap_or_default();
    let inputs = queue_inputs(world, &mut state, incoming);
    let online: HashMap<Entity, Entity> = world
        .query_filtered::<(Entity, &Player), With<Character>>()
        .iter(world)
        .map(|(actor, player)| (player.connection, actor))
        .collect();
    let mut cache = TickCache::default();
    for input in inputs {
        process_input(world, &mut state, &online, &mut cache, input);
    }
    let now = world.resource::<Time>().elapsed_secs_f64();
    if now >= state.next {
        state.next = now + 0.5;
        let connections: Vec<_> = world
            .query::<(Entity, &Connection, &TelnetCapabilities)>()
            .iter(world)
            .map(|(entity, _, caps)| (entity, caps.clone()))
            .collect();
        let alive: HashSet<_> = connections.iter().map(|(entity, _)| *entity).collect();
        state
            .clients
            .retain(|connection, _| alive.contains(connection));
        for (connection, caps) in connections {
            publish_connection(world, &mut state, &online, &mut cache, connection, &caps);
        }
    }
    world.insert_resource(state);
}

pub(super) fn update_owner(
    world: &mut World,
    connection: Entity,
    caps: &TelnetCapabilities,
    client: &mut ClientState,
    actor: Option<Entity>,
) {
    if actor != client.owner {
        if caps.gmcp && client.owner.is_some() {
            for name in ["Char.Vitals", "Char.Status", "Char.Stats", "Room.Info"] {
                gmcp(world, connection, name, &json!({}));
            }
            gmcp(
                world,
                connection,
                "Char.Items.List",
                &json!({"location":"inv","items":[]}),
            );
        }
        client.last.clear();
        client.owner = actor;
        client.last_inventory = Value::Null;
        client.links_shown = false;
    }
}

fn queue_inputs(
    world: &mut World,
    state: &mut State,
    incoming: Vec<TelnetInput>,
) -> Vec<TelnetInput> {
    for input in incoming {
        if world.get::<Connection>(input.connection).is_none() {
            continue;
        }
        let client = state.clients.entry(input.connection).or_default();
        if client.overloaded {
            continue;
        }
        if client.pending.len() >= INPUT_BACKLOG {
            client.pending.clear();
            client.overloaded = true;
            world.write_message(grim::ConnectionOutput::new(
                input.connection,
                "Too many pending protocol requests; reconnect to resynchronize.\n",
            ));
            world.write_message(grim::DisconnectRequest {
                connection: input.connection,
            });
        } else {
            client.pending.push_back(input);
        }
    }
    let mut inputs = Vec::new();
    for (connection, client) in &mut state.clients {
        if world.get::<Connection>(*connection).is_none() {
            client.pending.clear();
            continue;
        }
        let count = client.pending.len().min(INPUTS_PER_TICK);
        inputs.extend(client.pending.drain(..count));
    }
    inputs
}

#[derive(Default)]
struct TickCache {
    snapshots: HashMap<Entity, BTreeMap<String, Value>>,
    inventories: HashMap<Entity, Value>,
    inventory_replied: HashSet<Entity>,
}
fn process_input(
    world: &mut World,
    state: &mut State,
    online: &HashMap<Entity, Entity>,
    cache: &mut TickCache,
    input: TelnetInput,
) {
    if world.get::<Connection>(input.connection).is_none() {
        return;
    }
    let Some(caps) = world.get::<TelnetCapabilities>(input.connection).cloned() else {
        return;
    };
    let client = state.clients.entry(input.connection).or_default();
    update_owner(
        world,
        input.connection,
        &caps,
        client,
        online.get(&input.connection).copied(),
    );
    if input.option == MSSP && caps.mssp {
        mssp(world, input.connection);
        client.mssp_sent = true;
    }
    if input.option == GMCP && caps.gmcp {
        let text = String::from_utf8_lossy(&input.payload);
        if text.trim() == "Core.Ping" {
            protocol(world, input.connection, GMCP, b"Core.Ping".to_vec());
        }
        if text.starts_with("Core.Supports.") || input.payload.is_empty() {
            client.last.clear();
        }
        if let Some(actor) = online.get(&input.connection) {
            if text.trim() == "Char.Items.Inv" && cache.inventory_replied.insert(input.connection) {
                let items = cache
                    .inventories
                    .entry(*actor)
                    .or_insert_with(|| json!(inventory(world, *actor)));
                gmcp(
                    world,
                    input.connection,
                    "Char.Items.List",
                    &json!({"location":"inv","items":items}),
                );
            }
        }
    }
    if input.option == MSDP && caps.msdp {
        process_msdp(
            world,
            client,
            online.get(&input.connection).copied(),
            cache,
            &input,
        );
    }
}

fn process_msdp(
    world: &mut World,
    client: &mut ClientState,
    actor: Option<Entity>,
    cache: &mut TickCache,
    input: &TelnetInput,
) {
    for (command, values) in msdp_commands(&input.payload) {
        for value in values {
            match command.as_str() {
                "LIST" => {
                    let list = match value.as_str() {
                        "COMMANDS" => Some(json!(["LIST", "SEND", "REPORT", "UNREPORT", "RESET"])),
                        "LISTS" => Some(json!([
                            "COMMANDS",
                            "LISTS",
                            "REPORTABLE_VARIABLES",
                            "REPORTED_VARIABLES",
                            "SENDABLE_VARIABLES"
                        ])),
                        "REPORTABLE_VARIABLES" | "SENDABLE_VARIABLES" => Some(json!(REPORTABLE)),
                        "REPORTED_VARIABLES" => {
                            Some(json!(client.reported.iter().collect::<Vec<_>>()))
                        }
                        _ => None,
                    };
                    if let Some(list) = list {
                        protocol(world, input.connection, MSDP, msdp_pair(&value, &list));
                    }
                }
                "REPORT" if REPORTABLE.contains(&value.as_str()) => {
                    client.reported.insert(value);
                    client.last.clear();
                }
                "UNREPORT" => {
                    client.reported.remove(&value);
                }
                "RESET" => client.reported.clear(),
                "SEND" if REPORTABLE.contains(&value.as_str()) => {
                    if let Some(actor) = actor {
                        let vars = cache
                            .snapshots
                            .entry(actor)
                            .or_insert_with(|| variables(&snapshot(world, actor)));
                        if let Some(data) = vars.get(&value) {
                            protocol(world, input.connection, MSDP, msdp_pair(&value, data));
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn publish_connection(
    world: &mut World,
    state: &mut State,
    online: &HashMap<Entity, Entity>,
    cache: &mut TickCache,
    connection: Entity,
    caps: &TelnetCapabilities,
) {
    let client = state.clients.entry(connection).or_default();
    offer_map(world, connection, caps, client);

    if caps.mssp && !client.mssp_sent {
        mssp(world, connection);
        client.mssp_sent = true;
    }
    let actor = online.get(&connection).copied();
    update_owner(world, connection, caps, client, actor);
    let Some(actor) = actor else {
        return;
    };
    if !client.links_shown
        && world
            .get::<Character>(actor)
            .is_some_and(|c| c.config.get("client_links").is_some_and(|v| v == "on"))
    {
        client.links_shown = client_links(world, connection, caps);
    }
    let data = snapshot(world, actor);
    if caps.gmcp {
        for (package, value) in &data {
            if package_supported(caps, package) && client.last.get(package) != Some(value) {
                gmcp(world, connection, package, value);
                client.last.insert(package.clone(), value.clone());
            }
        }
        if package_supported(caps, "Char.Items.List") {
            let items = cache
                .inventories
                .entry(actor)
                .or_insert_with(|| json!(inventory(world, actor)));
            if *items != client.last_inventory {
                gmcp(
                    world,
                    connection,
                    "Char.Items.List",
                    &json!({"location":"inv","items":items}),
                );
                client.last_inventory = items.clone();
            }
        }
    }
    if caps.msdp {
        for (name, value) in variables(&data) {
            let key = format!("MSDP:{name}");
            if client.reported.contains(&name) && client.last.get(&key) != Some(&value) {
                protocol(world, connection, MSDP, msdp_pair(&name, &value));
                client.last.insert(key, value);
            }
        }
    }
    if caps.atcp && !caps.gmcp {
        if let Some(room) = data.get("Room.Info") {
            if client.last.get("ATCP:Room") != Some(room) {
                protocol(
                    world,
                    connection,
                    ATCP,
                    format!("Room.Brief {}", room["name"].as_str().unwrap_or("")).into_bytes(),
                );
                client.last.insert("ATCP:Room".into(), room.clone());
            }
        }
    }
}

fn offer_map(
    world: &mut World,
    connection: Entity,
    caps: &TelnetCapabilities,
    client: &mut ClientState,
) {
    if caps.gmcp && !client.map_offered {
        let settings = world.resource::<Settings>().clone();
        if settings.map_url.starts_with("https://")
            && !world.resource::<MapperIds>().rooms.is_empty()
        {
            gmcp(
                world,
                connection,
                "Client.Map",
                &json!({"url":settings.map_url}),
            );
            if settings.gui_url.starts_with("https://") {
                gmcp(
                    world,
                    connection,
                    "Client.GUI",
                    &json!({"version":settings.gui_version,"url":settings.gui_url}),
                );
            }
            client.map_offered = true;
        }
    }
}
