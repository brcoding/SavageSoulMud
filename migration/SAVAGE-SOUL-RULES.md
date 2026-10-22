# Savage Soul rules source and conversion baseline

Authoritative sources supplied by the user:
- [Soulgen GS 2026 1.10 workbook](https://docs.google.com/spreadsheets/d/1D-A0CpD1z3DozIOxiQbmPdlpuDv5uVzKbR4uehD8oGU/edit?gid=2002459905)
- [The Savage Soul 2026 rulebook](https://drive.google.com/file/d/1w0cxJmUqhapnWSmEu3GaVvK-NsX53WUH/view)

Downloaded and extracted locally on October 12, 2026. Reviewed the core character/task/combat sections; this is not a completed audit of every rulebook appendix.

## Findings

The character system spends XP on attributes and Peculiarities. Recommended initial XP is 600 (printed p.8, section 2.2.1; Soulgen Char.!B7). Attribute purchase costs and derived scores replace reliance on Grim's default race/class/level menus (chapter 2; Char.!A10:G29).

Task checks add a d10 to an attempt bonus, compare against difficulty, and retain success/failure margins. Attacks involve passive defense and eligible active defenses (sections 3.1.3–3.1.7.4). Combat has actions, reactions, and ready/motivation tokens (chapter 5). Armor, damage resistance, conditions, equipment, and magic require rule-specific implementation.

The rulebook identifies 1,000 starting credits and permits exchanging initial XP for credits at 100 credits per XP (section 2.2.1.1). Current Grim starts at 100 credits, a provisional local choice made before these rules were supplied. Existing balances have not been changed. Sam's stock/prices are provisional too.

## Implementation direction

Use these sources for character data and combat; use old LPC files for setting content and recovery clues. Keep credits as the user-requested single integer. Implement persisted attributes/XP/Peculiarities and derived character display first, then task resolution, equipment/armor, and combat actions/defenses/conditions. Preserve Rawr's identity, account, wizard role, inventory, and credits during migration. Explicitly document MUD adaptations of tabletop range, timing, and leadership; do not invent missing rules or claim placeholders already implement Savage Soul.
