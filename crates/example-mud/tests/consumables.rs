mod harness;
use harness::Mud;

#[test]
fn supplies_are_consumed_once_and_saved() {
    let mut mud=Mud::with_savage_soul();let(s,_)=mud.connect();
    for line in ["food@example.com","y","secretpw","c","Hungry","1",""]{let _=mud.send(s,line);}
    mud.send(s,"drink water").assert_contains("not carrying");
    for line in ["east","east","north","north","west"]{let _=mud.send(s,line);}
    let _=mud.send(s,"buy water");let _=mud.send(s,"buy ration");
    mud.send(s,"drink bottle").assert_contains("You drink the water");
    mud.send(s,"drink water").assert_contains("bottle is empty");
    mud.send(s,"eat food").assert_contains("eat the preserved food");
    let saved=mud.saved_character("Hungry");
    assert_eq!(saved.credits,992);assert_eq!(saved.inventory.iter().filter(|o|o.name=="empty water bottle").count(),1);
    assert!(saved.inventory.iter().any(|o|o.name=="empty water bottle"));
    let _=mud.send(s,"quit");mud.disconnect(s);mud.reboot();
    let(s,_)=mud.connect();for line in ["food@example.com","secretpw","1",""]{let _=mud.send(s,line);}
    mud.send(s,"inventory").assert_contains("empty water bottle").assert_excludes("ration pack");
    mud.send(s,"drink bottle").assert_contains("bottle is empty");
}
