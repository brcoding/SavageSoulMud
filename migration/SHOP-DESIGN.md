# Local shops

Problem: Trader Sam needs list/buy/sell with an integer credit balance.

Constraints checked: ARCHITECTURE.md acyclic dependencies and game-local gameplay extensions; CONTEXT.md actor/character vocabulary; ADR-0007 is proposed and does not authorize a shared-item redesign; grim-object README inventory ownership and snapshot persistence; grim-actor README sole StoredCharacter serialization surface; AGENTS.md command compatibility.

Decision: one game-local ShopPlugin and optional Merchant seed component. Use existing object/target primitives, Character/StoredCharacter credits, current command enum/registry integration. No new shared contract crate or proposed ADR implementation. Exclusive-world transactions validate all items/credits, save one whole character snapshot before ECS changes, and reject arithmetic overflow or save failures.

Retrofit: update character constructors and DTO conversion; NPC shop blueprint field; parser commands without displacing l/look or s/south; real binary and test harness both load ShopPlugin. Test only the trade and save integration relevant to this change.
