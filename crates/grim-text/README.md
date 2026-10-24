# `grim-text`
> The text catalog: author-facing strings addressed by key, with `%{var}` interpolation.

**Role:** horizontal (infrastructure) — pure functions
**Depends on:** `grim-color` (to convert markup and escape interpolated values). No Bevy, no i18n runtime.

## Components
None. Pure library — no Bevy dependency, no ECS types.

## Systems
None. No `Plugin`, no `add_systems`.

## Commands
None.

## Resources & Events
None.

## Notes
- `login.version` renders the optional binary-supplied build label at connection time.
- Public API (`src/lib.rs`): `tr(key, args)` and the `tr!` macro. Lookup converts colour markup via `grim-color::convert_16color`, then substitutes `%{name}` placeholders, escaping each value via `escape_codes` so a value can never inject markup. An unknown key resolves to the key itself, surfacing the miss.
- Layout helpers: `column_grid` (keyword grids) and `table(headers, rows, align)` (`src/table.rs`: header + dash separator + left/right-aligned rows for heterogeneous data like `sockets`; visible widths, so pass already-escaped display strings).
- Uses `%{var}` (not `{var}`) precisely because GRIM's colour markup already uses `{` — see the module docs and ARCHITECTURE.md §5.4.
- Defaults are **inlined** in `default_string`, not loaded from disk. The author-override `Catalog` resource that merges `strings/<locale>/*.json` + `templates/<locale>/` is deferred to the plugin-composition work (ARCHITECTURE.md §5.4, §8), which keeps this crate Bevy-free for now. This crate replaced two parallel string systems (`rust-i18n` + a hand-rolled `tr`). Thin today — improve over time.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*
