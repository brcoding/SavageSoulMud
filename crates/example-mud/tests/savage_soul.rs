mod harness;
use harness::Mud;

#[test]
fn soul_creation_training_equipment_and_restart_preserve_state() {
    let mut mud=Mud::with_savage_soul();
    let (s,_)=mud.connect();
    for input in ["soul@example.com","y","secretpw","c","Savage","1",""] {let _=mud.send(s,input);}
    mud.send(s,"look").assert_contains("Royal Oak Med Room");
    mud.send(s,"score").assert_contains("600 total, 600 unspent").assert_contains("Credits: 1000");
    mud.send(s,"train body").assert_contains("body is now 6");
    mud.send(s,"train faith").assert_contains("Faith cannot");
    mud.send(s,"learn unarmed fighter").assert_contains("Learned");
    assert_eq!(mud.saved_character("Savage").game_state["savage_soul"]["attribute_levels"]["body"],1);
    for input in ["east","east","north","north","west"] {let _=mud.send(s,input);}
    mud.send(s,"buy pocket knife").assert_contains("900 credits remaining");
    mud.send(s,"wield pocket knife").assert_contains("You wield Pocket Knife");
    mud.send(s,"buy light armor").assert_contains("400 credits remaining");
    mud.send(s,"wear light armor").assert_contains("You wear Light Armor");
    mud.send(s,"equipment").assert_contains("Pocket Knife").assert_contains("Light Armor");
    mud.send(s,"score").assert_contains("Armor DR 7");
    let _=mud.send(s,"quit");mud.disconnect(s);mud.reboot();
    let (s,_)=mud.connect();
    for input in ["soul@example.com","secretpw","1",""]{let _=mud.send(s,input);}
    mud.send(s,"score").assert_contains("600 total, 500 unspent").assert_contains("Credits: 400").assert_contains("Pocket Knife");
    mud.send(s,"remove armor").assert_contains("Equipment removed");
    mud.send(s,"score").assert_contains("Armor DR 6");
}

#[test]
fn leadership_combat_and_wizard_recovery_use_persistent_conditions() {
    let mut mud=Mud::with_savage_soul();
    let (s,_)=mud.connect();
    for input in ["combat@example.com","y","secretpw","c","Duelist","1",""] {let _=mud.send(s,input);}
    mud.edit_character("Duelist",|c|c.roles.push(grim::Role::Admin));
    for input in ["east","east","north","north","west"]{let _=mud.send(s,input);}
    mud.send(s,"combatlog detailed").assert_contains("Saved");
    mud.send(s,"attack sam").assert_contains("Combat begins").assert_contains("Initiative:").assert_contains(" vs ");
    mud.send(s,"stop").assert_contains("You stop attacking");
    mud.send(s,"soulheal self").assert_contains("Conditions restored and saved");
    assert_eq!(mud.saved_character("Duelist").game_state["savage_soul"]["injury"],"uninjured");
    mud.send(s,"awardxp self 100").assert_contains("XP awarded and saved");
    mud.send(s,"score").assert_contains("700 total");
}
