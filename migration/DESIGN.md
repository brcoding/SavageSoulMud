# Savsoul conversion design

Problem: run a local GRIM world with the existing Dead Souls Royal Oak content,
reachable from the default Haven start.

Constraints checked: docs/ARCHITECTURE.md (binary composes; retain plugin boundaries),
CONTEXT.md (canonical blueprints, stable identity), ADR-0001 (12-character base62
IDs and canonical cross-area exit resolution), example-mud README/seed.rs
(filesystem JSON seeds), AGENTS.md (test changes and keep owning documentation current).
No subsystem or ADR change is needed.

Decision: data conversion plus an optional scenery blueprint list, spawning named
inspectable entities with InRoom but no Object marker. Existing look handles these;
pickup naturally excludes them. Use reciprocal west/east door directions to support
the existing synchronized door behavior. Preserve unsupported LPC in an archive.
Keep the initial room in Haven and use WSL for upstream's Unix-only transport.

Validation: parse and graph checks; seed unit tests; unchanged vanilla scenarios;
Savsoul integration scenario covering door blocking/open/close, both directions,
medical signs/table, trail and bar second-floor routes, and immovable fixtures.

Scope: local installation and testing. No GitHub publication or remote hosting.
