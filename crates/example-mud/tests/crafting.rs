mod harness;
use harness::Mud;

#[test]
fn recovered_cooking_consumes_exact_supplies_and_saves_food() {
    let mut mud=Mud::with_savage_soul();
    let(s,_)=mud.connect();
    for line in ["crafter@example.com","y","secretpw","c","Crafter","1",""] {let _=mud.send(s,line);}
    mud.edit_character("Crafter",|c|c.roles.push(grim::Role::Admin));
    for line in ["east","east","north","north","west"] {let _=mud.send(s,line);}
    mud.send(s,"buy pocket knife").assert_contains("You buy");
    mud.send(s,"goto savsoul:robarngrill01").assert_contains("Royal Oak Bar N Grill");
    mud.send(s,"recipes ham sandwich").assert_contains("Ingredients:").assert_contains("Tools: knife");
    mud.send(s,"buy 2*white bread").assert_contains("You buy");
    mud.send(s,"buy 2*slice of pork").assert_contains("You buy");
    mud.send(s,"craft ham sandwich").assert_contains("You craft ham sandwich");
    let saved=mud.saved_character("Crafter");
    assert!(saved.inventory.iter().any(|i|i.keywords.iter().any(|k|k=="craft:ham-sand")));
    assert!(!saved.inventory.iter().any(|i|i.keywords.iter().any(|k|k=="craft:piece-of-white-bread")));
    let count=saved.inventory.len();
    mud.send(s,"craft ham sandwich").assert_contains("You need");
    assert_eq!(mud.saved_character("Crafter").inventory.len(),count);
    let _=mud.send(s,"quit");mud.disconnect(s);mud.reboot();
    let(s,_)=mud.connect();
    for line in ["crafter@example.com","secretpw","1",""]{let _=mud.send(s,line);}
    mud.send(s,"eat ham sandwich").assert_excludes("cannot eat");
    assert!(!mud.saved_character("Crafter").inventory.iter().any(|i|i.keywords.iter().any(|k|k=="craft:ham-sand")));
}

#[test]
fn metalshop_batches_and_reductions_use_exact_instances() {
    let mut mud=Mud::with_savage_soul();let(s,_)=mud.connect();
    for line in ["smith@example.com","y","secretpw","c","Smith","1",""] {let _=mud.send(s,line);}
    mud.edit_character("Smith",|c|c.roles.push(grim::Role::Admin));
    mud.send(s,"goto southfield:southfieldmetalshop").assert_contains("Metalshop");
    mud.send(s,"buy sheet of steel").assert_contains("You buy");
    mud.send(s,"craft gh3 shell").assert_contains("Produced: 6");
    let count=|mud:&Mud,id:&str|mud.saved_character("Smith").inventory.iter().filter(|i|i.keywords.iter().any(|k|k==id)).count();
    assert_eq!(count(&mud,"craft:gh3-shell"),6);
    for line in ["buy 3*gram of gunpowder","buy chunk of lead","buy pliers"] {mud.send(s,line).assert_contains("You buy");}
    mud.send(s,"craft gh3 round").assert_contains("You craft");
    assert_eq!(count(&mud,"craft:gh3-shell"),5);
    assert_eq!(count(&mud,"craft:gh3-round"),1);
    mud.send(s,"reduce gh3 round").assert_contains("You reduce");
    assert_eq!(count(&mud,"craft:gh3-round"),0);
    assert_eq!(count(&mud,"craft:gh3-shell"),6);
    assert_eq!(count(&mud,"craft:gram-of-gunpowder"),3);
    mud.send(s,"reduce gh3 round").assert_contains("not carrying");
    mud.send(s,"craft wood handle").assert_contains("no bandsaw");
}

#[test]
fn recovered_morlock_lenses_open_panel_and_save_reward() {
    let mut mud=Mud::with_savage_soul();let(s,_)=mud.connect();
    for line in ["lens@example.com","y","secretpw","c","Lens","1",""] {let _=mud.send(s,line);}
    mud.edit_character("Lens",|c|c.roles.push(grim::Role::Admin));
    let _=mud.send(s,"goto savsoul:morlockcavemachine01");
    let _=mud.send(s,"defend none");
    let initial=mud.saved_character("Lens").game_state["savage_soul"]["total_xp"].as_i64().unwrap();
    for (color,count) in [("red",5),("blue",3),("green",3),("yellow",6)] {for _ in 0..count {mud.send(s,&format!("press {color} button")).assert_contains("button clicks");}}
    mud.send(s,"press yellow button").assert_contains("XP +100");
    assert_eq!(mud.saved_character("Lens").game_state["savage_soul"]["total_xp"].as_i64().unwrap(),initial+100);
    mud.send(s,"get suit from panel").assert_contains("take the white NBC suit");
    mud.send(s,"get all from panel").assert_contains("empty");
    assert_eq!(mud.saved_character("Lens").inventory.iter().filter(|i|i.name=="nbc suit").count(),1);
}
