Wizard browser editor

Open http://localhost:4080 on the computer hosting the MUD.
Log in with your wizard character name or account email and MUD password.
Only wizard/admin characters may use the editor.

Browse World to edit rooms, NPCs, shop stock, objects, scenery, exits and doors.
Expand nested fields. Source edits the complete JSON. Other categories cover
help, rules, quests, socials, migration overrides, MOTD and other text assets.
New file creates a draft. Add a help index entry when creating a help topic.

Validate checks a draft. Save changes writes it with a backup. History restores
prior versions. Conflicting external edits are refused. Help and MOTD are read
on their next use. World changes need Apply world changes: connections survive,
but fights, NPC positions and ground items reset. Embedded assets need a rebuild.
Character saves and passwords are not editable. The service is savsoul-editor.

World atlas opens the illustrated region index. Choose an area and Map for
room-level views. Fantasy atlas, TTRPG battle grid and Exit blueprint styles
support pan/zoom and separate floors. The map shows unsaved drafts and flags
broken exits, inconsistent returns, overlaps and conflicting positions.
Use named exit selectors to edit links. See help editor-map.
