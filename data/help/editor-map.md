Wizard atlas and exit diagnostics

Open http://localhost:4080 and log in as a wizard.
World atlas shows regions; click a region to inspect its actual room links.
An area file also has a Map tab. Choose Fantasy atlas, Hex grid or
Exit blueprint. Region / Town / Room set zoom levels. Drag to pan, wheel to
zoom, or use Fit. Arrow keys pan; plus/minus zoom; F fits the current map.

Floor selects ground, upper or basement levels inferred from up/down exits,
or shows all floors in separate panels. Click a room to edit its full fields
in the right-hand inspector, including descriptions, exits and nested settings.
Drag the divider or use the Room inspector width slider to resize the panes.
The divider also accepts arrow keys, Home and End. Your width is remembered
in this browser; narrow windows stack the panes vertically.
The map uses your current unsaved draft. Save changes in the inspector or
toolbar saves the entire area, using the same validation and backup process.
Artwork and coordinates are schematic: these rooms do not have surveyed
floor dimensions, actual object positions or known continent coastlines.

Named exit selectors replace cryptic destination IDs. Choose a room, add an
exit, or explicitly add a missing return within the same area file. Existing
returns are preserved. Cross-area returns are edited in their own files.
Save changes backs up and validates content; Apply reloads the world.

Diagnostics distinguish broken targets from warnings about wrong returns,
door differences, overlapping rooms and conflicting directional loops.
One-way exits and line crossings are notes; they may be intentional. Toggle
notes to show them. Click a diagnostic to locate the room and its floor.
No map diagnostic automatically rewrites the MUD.
