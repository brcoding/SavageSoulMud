//! Transport-neutral credential handoff. Never route secrets through command input.
use bevy::prelude::*;
use grim_actor::{Actor,Character};
use grim_core::{components::{Account,Name as GrimName},GrimId};
use grim_persistence::{load_character_by_name,PersistenceConfig};

#[derive(Message)]
pub struct LoginCredentials {pub connection:Entity,pub account:String,pub password:String}
impl std::fmt::Debug for LoginCredentials {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.debug_struct("LoginCredentials").field("connection",&self.connection).field("credentials",&"[redacted]").finish()}
}
#[derive(Message,Clone,Debug)]
pub struct LoginResult {pub connection:Entity,pub success:bool,pub message:String}
#[derive(SystemSet,Debug,Clone,PartialEq,Eq,Hash)]
pub enum AuthSystems {Greet,Input}

pub(crate) fn resolve(account:&str,accounts:&Query<(Entity,&mut Account)>,characters:&Query<(Entity,&Character,&Actor,&GrimName)>,persistence:&PersistenceConfig)->Option<(String,Option<String>)> {
    let character=|name:&str|->Option<(GrimId,String)> {
        crate::validation::validate_character_name(name).ok()?;
        let name=crate::validation::normalize_character_name(name);
        characters.iter().find(|(_,_,_,n)|n.0==name).map(|(_,c,_,n)|(c.account_id,n.0.clone()))
            .or_else(||load_character_by_name(persistence,&name).map(|c|(c.account_id,c.name)))
    };
    if let Some((identifier,name))=account.split_once(':') {
        let (_,owner)=accounts.iter().find(|(_,a)|a.identifier.eq_ignore_ascii_case(identifier))?;
        let (id,name)=character(name)?;
        return (id==owner.id).then(||(owner.identifier.clone(),Some(name)))
    }
    if let Some((_,a))=accounts.iter().find(|(_,a)|a.identifier.eq_ignore_ascii_case(account)) {return Some((a.identifier.clone(),None))}
    let (id,name)=character(account)?;
    accounts.iter().find(|(_,a)|a.id==id).map(|(_,a)|(a.identifier.clone(),Some(name)))
}
