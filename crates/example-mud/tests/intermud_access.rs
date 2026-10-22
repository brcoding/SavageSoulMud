mod harness;
use harness::Mud;

#[test]
fn every_intermud_alias_is_masked_for_mortals() {
    let mut mud=Mud::with_savage_soul();let(s,_)=mud.connect();
    for input in ["mortal-i3@example.com","y","secretpw","c","Mortal","1",""] {let _=mud.send(s,input);}
    for input in ["i3 status","i3 recent","i3 activity","i3 listen all","i3 channels","i3 tell friend@Mud hello","intermud status","imud intergossip hello", "zebedee status", "i2 status", "grapevine status", "gv recent", "gv listen gossip", "gv chat testing hello", "gv tell friend@Game: hello", "gv reload"] {
        mud.send(s,input).assert_contains("Unknown command");
    }
    let commands=mud.send(s,"commands");
    for name in ["i3","intermud","imud","zebedee","i2","grapevine","gv"] {assert!(!commands.text().split_whitespace().any(|word|word==name),"{}",commands.text());}
    mud.send(s,"help intermud").assert_excludes("Intermud connects");
    mud.send(s,"help grapevine").assert_excludes("Grapevine is separate");
}
