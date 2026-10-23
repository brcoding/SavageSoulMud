# The Savage Soul MUD

A recovered Savage Soul world running on a customized [GRIM](https://github.com/gvanderest/grim)
Rust/Bevy engine. This repository stores the engine adaptations, converted world,
authored rules/help, wizard editor, client support and deployment tools.

## Play

- MUD: `savsoul.com:4000`
- Secure MUD: `savsoul.com:4001` (TLS)
- Website and browser terminal: https://savsoul.com/
- Wizard editor: https://savsoul.com:4081/ (wizard MUD login required)

## Included

Savage Soul XP-based characters, species templates, combat, shops and credits,
crafting, medical services and paid character rebuilding; restored rooms, NPCs,
puzzles and gates; accessible output, combat detail options and prompts;
wizard editing and map inspection; Mudlet saved sign-in and downloadable maps;
I3, Zebedee and Grapevine integrations.

## Build and configure

Install the Rust toolchain specified in rust-toolchain.toml. Build on Linux/WSL:

Run the executable from Linux storage, including under WSL. Do not execute it
from a network-backed drive such as P:; `start-local.sh` creates a private Linux runtime image.

```sh
cargo build --locked --release -p example-mud --bin savsoul
cargo run --locked -p example-mud --bin savsoul
```

The plain MUD listens on port 4000. TLS and supporting web/network services have
separate setup under tools/. External MUD networks are disabled in this checkout
until configured with private credentials and a suitable unique development
identity. Never use the production network identity for a second running copy.
Copy `data/{intermud,grapevine,zebedee}.example.json` to the corresponding
untracked `.json` filename before configuring an adapter. Keep credentials
outside those files in private state; the examples contain disabled adapters.

Authored content is tracked under data/areas, data/help, data/quests,
data/savage-soul and data/socials. Accounts and player saves are created locally
under data/accounts and data/characters and are intentionally ignored.
Preserve data/mudlet-map-ids.json to keep Mudlet room numbers stable.

## Documentation

- [Engine and game components](crates/example-mud/README.md)
- [Wizard editor](tools/wizard-editor/README.md)
- [Mudlet authentication and maps](tools/mudlet/README.md)
- [Recovery and conversion notes](migration/)
- [Engine architecture](docs/ARCHITECTURE.md)
- [Original GRIM README](docs/GRIM-UPSTREAM-README.md)
- [Attribution and unresolved licensing](UPSTREAM.md)

This is a source/configuration backup, not a backup of private production state.
Keep server credentials and player-data backups separately.

The initial CI checks compilation, character rules, saved sign-in and website
building. Production deployment is not automatic. The inherited GRIM EC2
workflow is retained only as deploy/upstream-deploy.yml.example; it is not the
Savage Soul server deployment configuration.
