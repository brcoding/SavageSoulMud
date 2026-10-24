# Hidden test accounts and wizard invisibility

Create a separate ordinary MUD account and character using normal login. From
an existing wizard, run `testaccount CharacterName on`. This flags the entire
account, including future characters, as a hidden testing identity. It does not
grant wizard privileges. Only a wizard can remove it with
`testaccount CharacterName off`; testers cannot reveal themselves with wizinvis.
Keep the testing account's password separate from administrator credentials.

Wizards can use `wizinvis` to toggle their own visibility, or `wizinvis on` /
`wizinvis off` explicitly. The preference survives normal saves/relogin/copyover;
it only applies while the character has the admin role. Test-account status is
saved immediately on the account. Its character marker is reconciled from that
account before public scene output and input, including startup and copyover.

Ordinary players cannot see hidden identities in room presence, who, where,
wizlist or finger (including offline fallback). Hidden movement, login/logout,
speech, socials, item actions, doors, recall and attributed puzzle/table reactions
are silent to them. Hidden actors cannot privately message, give to or steal from
ordinary players while hidden. Target ranking excludes hidden candidates first.
Self and other wizards can still see hidden staff for supervision and auditing.
The tester role alone does not grant admin commands, network chat or editor login.

Public I3/Zebedee listings and Grapevine presence omit hidden wizards. Outgoing
network commands are unavailable while invisible, so those networks cannot expose
a hidden sender. Wizinvis does not conceal staff from administrator audit logs.

This is visibility control in the shared world, **not a private test instance**.
Testing can still change shared NPCs, items, shops and quest state. Use a separate
development server for destructive tests or isolated world-state experiments.

Implementation: `Account.testing` is a serde-default flag, mirrored to
`Role::Tester`. `Character::is_hidden` / `visible_to` and actor `Visibility`
define perception; scene renderers apply it per recipient. Delayed/scripted
responses retain their initiating visibility. Session staff commands live in
`grim-scene/src/staff.rs`. Account creation never accepts a client-supplied test
flag, and the wizard permission gate is repeated in the command handler.
