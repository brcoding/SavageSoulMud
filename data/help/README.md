# Editing the in-game help library

The deployed library is `P:\savsoulmud\grim\data\help`. This directory is the review copy. Edit the deployed files to update the running game.

The server reads `index.json` and topic files on each help request. No build, restart or copyover is needed for text changes. Keep a backup when making larger edits.

## Change an existing topic

Find its entry in `index.json`, then edit the named `.md` file. Use readable plain text: headings, blank lines, short paragraphs and indented command examples. These files display as text in the MUD; Markdown links and rich rendering are not required.

## Add a topic

Create `fishing.md`, then add an object to the JSON array:

```json
{
  "id": "fishing",
  "title": "Fishing",
  "category": "Guides",
  "aliases": ["fish", "angling"],
  "wizard": false,
  "file": "fishing.md"
}
```

Use unique, case-insensitive IDs and aliases. Keep files within this directory. `commands`, `topics` and `search` are reserved help switches. Topic names can contain hyphens; aliases can contain spaces. Set `wizard` to true for administrative help. That flag hides documentation from ordinary players; it does not grant or revoke command permissions.

Categories `Guides`, `Exploration`, `Items`, `Shops`, `Savage Soul`, `Social`, `Interface` and `Wizard` group existing entries. Guides are general explanations; the other categories primarily document commands.

After editing, try `help fishing`, `help topics` and `help search fishing` in the game. Check JSON commas and quotes if the library reports a loading error. Removing an index entry removes that topic from browsing and lookup; the unused file can remain until you choose to delete it.

Document current behavior. Mark unfinished systems explicitly and update related help when commands change. Do not add player account data, passwords or private operational notes to help files.
