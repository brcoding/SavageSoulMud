# `grim-social`
> Data-driven socials (`grin`, `smile`, `wave`, …): emote commands with per-audience message variants.

**Role:** vertical — player expression / emotes
**Depends on:** `grim-core`, `grim-actor`, `grim-text`, `grim-target`, `grim-command`

## Components
None — socials carry no components.

## Systems
| System | Schedule | File | Purpose |
|---|---|---|---|
| `load_and_register` | `Startup` | `src/plugin.rs` | Loads built-ins + `data/socials/*.json`, registers each name into the shared `CommandRegistry` (deprioritized; exact static collisions skipped). |
| `handle_social` | `Update` | `src/handler.rs` | Reads `Command::Social`/`SocialList`, resolves the target against the actor's roommates, renders per-recipient `InfoMessage`s, emits `SocialPerformed`. |

## Commands
Player-facing verbs and where to find their handlers.
| Command | Handler | Summary |
|---|---|---|
| `<social>` | `handle_social` (`src/handler.rs`) | Solo wording to the actor, solo room wording to roommates (e.g. `grin` → "You grin." / "Alice grins."). |
| `<social> <who>` | `handle_social` (`src/handler.rs`) | Actor / target / room wordings (e.g. `grin bob` → "You grin at Bob." / "Alice grins at you." / "Alice grins at Bob."). Target resolves by `grim-target` rank among roommates; `self` (or your own name) uses the self case. |
| `<social> self` | `handle_social` (`src/handler.rs`) | Self case (e.g. `grin self` → "You grin to yourself." / "Alice grins to himself."). |
| `socials` | `handle_social` (`src/handler.rs`) | Grid of the data-driven socials (the `"social"` registry section). |

Built-ins (14): grin, smile, wave, laugh, nod, bow, chuckle, cry, dance, hug, kiss, shrug, sigh, wink.

## Resources & Events
| Name | Kind (Resource/Message) | File |
|---|---|---|
| `SocialDir` | Resource | `src/social.rs` | Override directory (default `data/socials`, CWD-relative). |
| `SocialRegistry` | Resource | `src/social.rs` | All loaded socials by lowercase name. |
| `SocialDef` | Type | `src/social.rs` | One social: name + per-variant file overrides over catalog defaults. |
| `SocialPerformed` | Message (fact) | `src/handler.rs` | A social rendered (`actor`, `name`, `target`). For logging/moderation; nothing reads it yet. |
| `EngineCommand` | Message (input, from `grim-core`) | `src/handler.rs` |
| `InfoMessage` | Message (output, from `grim-core`) | `src/handler.rs` |

## Configuration
One file per verb: `data/socials/<name>.json` (filename == command, lowercased).
Every key optional — a missing key inherits the built-in default, so a partial
file reskins two wordings and inherits the other five:

```json
{
  "solo": { "actor": "You grin widely.\n", "room": "%{actor} grins widely.\n" },
  "self_target": { "actor": "You grin to yourself.\n", "room": "%{actor} grins to %{actor.self}.\n" },
  "with_target": {
    "actor": "You grin at %{target}.\n",
    "target": "%{actor} grins at you.\n",
    "room": "%{actor} grins at %{target}.\n"
  }
}
```

Only `%{actor}` / `%{target}` (names) plus per-party pronouns substitute
(values are colour-escaped like every `tr` value). Templates are authored
male-assumed and translated off each party's `Actor` gender:

| Var | Male | Female | Neutral |
|---|---|---|---|
| `%{P.he}` | he | she | they |
| `%{P.him}` | him | her | them |
| `%{P.his}` | his | her | their |
| `%{P.self}` | himself | herself | themselves |

with P in `actor`/`target` (e.g. `%{actor.self}`, `%{target.him}`). Copying a
file under a new name adds a verb; a bad file is logged and skipped. Catalog
keys `social.<name>.<case>.<audience>` hold the defaults.

## Notes
- Names register **after** the static commands and are deprioritized, so
  statics keep their prefixes; an exact static collision skips the social
  (warned, statics win). Only new contested prefixes involving a social are
  logged here (the static set is already reported by `grim-scene`).
- Rendering is per-recipient `InfoMessage`s (the `tell` pattern), not a shared
  broadcast — the same fan-out the future `act()` extraction (#58) absorbs.
- Socials load once at startup; they are not re-read while the server is up —
  the deploy mirrors `data/socials` to the host (see `docs/DEPLOY.md`).
- Mobs and linkdead PCs are valid targets/audiences (roommates, not just
  online PCs); delivery to offline recipients follows the `InfoMessage` path.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

## Staff testing visibility

Social target/audience resolution excludes imperceptible participants before resolving names or rendering messages.

See [staff testing](../../docs/STAFF-TESTING.md) for permissions and shared-world limitations.
