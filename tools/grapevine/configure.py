#!/usr/bin/env python3
"""Save existing Grapevine game credentials in private local Linux state."""
import getpass
import json
import os
from pathlib import Path
import sys
import tempfile


def save_credentials(path, client_id, client_secret):
    path = Path(path)
    values = {"client_id": client_id.strip(), "client_secret": client_secret.strip()}
    for label, value in values.items():
        if not value or len(value) > 512 or any(char.isspace() or ord(char) < 32 for char in value):
            raise ValueError(f"{label} must be a nonempty credential without spaces or controls")
    parent = path.parent
    if parent.is_symlink() or path.is_symlink():
        raise ValueError("Refusing a symlink credential directory/file")
    parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    parent.chmod(0o700)
    fd, temporary = tempfile.mkstemp(prefix=".credentials-", dir=parent)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(values, stream)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main():
    if os.name != "posix" or not sys.stdin.isatty():
        raise SystemExit("Run this in an interactive WSL/Linux terminal; the secret will not be echoed.")
    directory = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state")) / "savsoul-grapevine"
    print("Enter the Client ID and Client Secret for your existing Grapevine game.")
    client_id = input("Client ID: ")
    client_secret = getpass.getpass("Client Secret (hidden): ")
    try:
        save_credentials(directory / "credentials.json", client_id, client_secret)
    except (ValueError, OSError) as error:
        raise SystemExit(f"Credentials were not saved: {error}") from None
    print("Private credentials saved. In the MUD, use: grapevine reload")


if __name__ == "__main__":
    main()
