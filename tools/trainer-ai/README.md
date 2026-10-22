# Local trainer translation

Players simply type `ask tully <question>` or `talk to tully <question>`.
The normal `ask tully for help` also works. The local Qwen3-0.6B model maps
questions to typed training topics and allowlisted attribute/Peculiarity names.
The model never writes player-facing prose, runs commands, or changes stats.
Answers are computed by `trainer.rs` from current SoulData when delivery occurs.
Explicit attribute/Peculiarity names, attribute definitions, fatigue, lists and
capacity questions use direct topic matching before inference. The model handles
other phrasing; it cannot override an explicitly named topic. Replies give a
short definition or effect and the relevant personal cost. Lists, capacity and
cost formulas are separate answers rather than one combined training lecture.
Only the question and previous attribute context are sent to the loopback model;
account names, passwords and character records are not sent. No cloud API is used.

## One-time setup (WSL Ubuntu 24.04)

    python3 tools/trainer-ai/install-local.py

The installer downloads the pinned official llama.cpp CPU build b11723 (~18 MB)
and official Qwen/Qwen3-0.6B-GGUF Q8_0 (~640 MB), verifies both SHA256 hashes,
and enables the WSL user service `savsoul-trainer`. Files remain under
`~/.local/share/savsoul-trainer`; no administrator install or GPU setup is needed.
The model listens only on 127.0.0.1:4091, uses four CPU threads and one inference
slot, and has a 2 GB service memory limit. Setup needs internet; questions do not.
The user service follows the same WSL user session as the local editor.

    systemctl --user status savsoul-trainer
    systemctl --user stop savsoul-trainer
    systemctl --user start savsoul-trainer

`data/savage-soul/trainer-ai.json` controls enabled/port/timeout_ms. Set enabled
false and copyover to use only the scripted fallback. JSON changes need copyover;
the model service can restart independently. Timeout is clamped to 0.5–8 seconds.
Runtime/model versions and hashes are pinned in install-local.py; no automatic
downloads or upgrades occur during game startup.

One bounded worker handles inference outside Bevy. One question per character
can be pending, with at most four queued. Queue overflow, unavailable service,
timeout, invalid JSON or unknown intent falls back to built-in interpretation.
Connection failures trigger a 20-second backoff. Async responses require the same
connected session, NPC and room; leaving or disconnecting discards the response.
The game accepts only enums and known subjects; constrained JSON is a formatting
aid, not proof that a translation is semantically correct. Players can rephrase.
Adding supported answers belongs in trainer.rs; update the allowlist and prompt
in trainer_ai.rs when adding a new translated topic.

Sources: https://huggingface.co/Qwen/Qwen3-0.6B-GGUF and
https://github.com/ggml-org/llama.cpp/releases/tag/b11723
