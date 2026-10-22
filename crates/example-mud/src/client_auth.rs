//! GMCP Char.Login v1 adapter; secrets never enter the text/command pipeline.
use bevy::{prelude::*,ecs::message::MessageCursor};
use grim::{AuthSystems,ClientState,Connection,LoginCredentials,LoginResult,TelnetCapabilities,TelnetInput,TelnetOutput};
use grim::components::Client;
use std::collections::HashSet;
use serde_json::json;
#[derive(Resource,Default)]
struct Offered(HashSet<Entity>);
pub(crate) fn register(app:&mut App) {
    app.init_resource::<Offered>().add_systems(Update,receive.after(AuthSystems::Greet).before(AuthSystems::Input))
        .add_systems(Update,results.after(AuthSystems::Input).before(grim::TelnetSystems::Send));
}
fn send(world:&mut World,connection:Entity,package:&str,value:serde_json::Value) {
    world.write_message(TelnetOutput{connection,option:201,payload:format!("{package} {value}").into_bytes()});
}
fn receive(world:&mut World,mut cursor:Local<MessageCursor<TelnetInput>>) {
    let inputs:Vec<_>=cursor.read(world.resource::<Messages<TelnetInput>>()).filter(|i|i.option==201).cloned().collect();
    let alive:HashSet<_>=world.query_filtered::<Entity,With<Connection>>().iter(world).collect();
    world.resource_mut::<Offered>().0.retain(|e|alive.contains(e));
    // Negotiation updates capabilities through deferred ECS commands. Poll the
    // settled connection state too, so the initial offer cannot be lost if the
    // first GMCP event arrived before that component update was visible.
    let sessions:Vec<_>=world.query::<&Client>().iter(world).filter(|c|c.state==ClientState::LoginPrompt).map(|c|c.connection).collect();
    for connection in sessions {
        if world.get::<TelnetCapabilities>(connection).is_some_and(|c|c.gmcp)
            && world.resource_mut::<Offered>().0.insert(connection) {
            send(world,connection,"Char.Login.Default",json!({"version":1,"type":["password-credentials"]}));
        }
    }
    for input in inputs {
        if !world.get::<TelnetCapabilities>(input.connection).is_some_and(|c|c.gmcp){continue;}
        let state=world.query::<&Client>().iter(world).find(|c|c.connection==input.connection).map(|c|c.state.clone());
        let Some(state)=state else{continue;};
        if state==ClientState::LoginPrompt && world.resource_mut::<Offered>().0.insert(input.connection) {
            send(world,input.connection,"Char.Login.Default",json!({"version":1,"type":["password-credentials"]}));
        }
        let text=String::from_utf8_lossy(&input.payload);
        let Some(body)=text.strip_prefix("Char.Login.Credentials ") else{continue;};
        let value=serde_json::from_str::<serde_json::Value>(body);
        if value.as_ref().is_ok_and(|v|v.as_object().is_some_and(|o|o.is_empty())){continue;}
        let valid=value.ok().filter(|v|v.get("version").is_none_or(|n|n==1||n=="1"));
        let pair=valid.as_ref().and_then(|v|Some((v.get("account")?.as_str()?,v.get("password")?.as_str()?)));
        if let Some((account,password))=pair {
            world.write_message(LoginCredentials{connection:input.connection,account:account.into(),password:password.into()});
        }else{send(world,input.connection,"Char.Login.Result",json!({"success":false,"message":"Use Char.Login version 1 with account and password, or {} for interactive login."}));}
    }
}
fn results(world:&mut World,mut cursor:Local<MessageCursor<LoginResult>>) {
    let replies:Vec<_>=cursor.read(world.resource::<Messages<LoginResult>>()).cloned().collect();
    for reply in replies {send(world,reply.connection,"Char.Login.Result",if reply.success{json!({"success":true})}else{json!({"success":false,"message":reply.message})});}
}
