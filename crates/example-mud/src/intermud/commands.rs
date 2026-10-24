//! commands implementation, split from the parent subsystem.

use super::*;

pub(super) fn request(
    bridge: &mut Bridge,
    name: &str,
    method: &str,
    params: Value,
) -> Result<(), String> {
    if !bridge.connected {
        return Err("Intermud gateway is disconnected. It will reconnect automatically.".into());
    }
    if bridge.pending.len() >= 64 {
        return Err("Intermud is busy. Try again shortly.".into());
    }
    bridge.next = bridge.next.wrapping_add(1).max(100);
    let id = bridge.next;
    let label = if method == "channel_list" && params["bulk_listen"] == true {
        "channel_list#listen_all".into()
    } else if method == "mudlist" {
        format!("mudlist#{}", params["local_filter"].as_str().unwrap_or(""))
    } else if method == "tell" {
        format!(
            "tell#{}@{}",
            params["target_user"].as_str().unwrap_or("?"),
            params["target_mud"].as_str().unwrap_or("?")
        )
    } else if method == "channel_send" {
        format!("channel_send#{}", params["channel"].as_str().unwrap_or("?"))
    } else {
        method.into()
    };
    bridge
        .tx
        .try_send(Request {
            id,
            method: method.into(),
            params,
        })
        .map_err(|_| "Intermud is busy. Try again shortly.".to_string())?;
    if !name.is_empty() || matches!(method, "channel_join" | "channel_leave") {
        bridge
            .pending
            .insert(id, (name.into(), label, Instant::now()));
    }
    Ok(())
}

pub(super) fn parse(name: &str, arg: &str) -> Result<(String, Value), String> {
    let (verb, rest) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
    let rest = rest.trim();
    let result=match verb.to_ascii_lowercase().as_str(){
        ""|"status"=>("status",json!({})),"muds"=>("mudlist",json!({"filter":{"status":"up"},"local_filter":rest})),"channels"=>("channel_list",json!({})),
        "listen" if rest.eq_ignore_ascii_case("all")=>("channel_list",json!({"bulk_listen":true})),
        "listen"|"unlisten" if valid_channel(rest)=>(if verb.eq_ignore_ascii_case("listen"){"channel_join"}else{"channel_leave"},json!({"channel":rest,"user_name":name})),
        "who" if !rest.is_empty()&&rest.len()<=100=>("who",json!({"target_mud":rest,"from_user":name})),
        "chat"=>{let(channel,message)=rest.split_once(' ').ok_or("Use intermud chat <channel> <message>.")?;if !valid_channel(channel)||message.trim().is_empty()||message.len()>1024{return Err("Use a channel name and a message of up to 1024 characters.".into());}("channel_send",json!({"channel":channel,"from_user":name,"message":message.trim()}))},
        "tell"=>{let(target,message)=rest.split_once(' ').ok_or("Use intermud tell <user>@<mud> <message>; mud names may contain spaces when separated with a colon: user@Mud Name: message.")?;
            let (target,message)=if let Some((target,message))=rest.split_once(": "){(target,message)}else{(target,message)};
            let(user,mud)=target.split_once('@').ok_or("Use intermud tell user@mud message.")?;if user.is_empty()||mud.is_empty()||message.trim().is_empty()||message.len()>1024{return Err("Use intermud tell user@mud message (maximum 1024 characters).".into());}("tell",json!({"target_user":user,"target_mud":mud,"from_user":name,"message":message.trim()}))},
        _=>return Err("Intermud: status, muds, channels, recent [channel], activity, listen <channel>, unlisten <channel>, chat <channel> <message>, tell user@mud message, who <mud>. See help intermud.".into())};
    Ok((result.0.into(), result.1))
}
