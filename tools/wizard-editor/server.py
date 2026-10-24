"""Local wizard content editor. Run with Waitress, never Flask's dev server."""
import collections
import datetime as dt
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import tempfile
import threading
import time

from flask import Flask, abort, g, jsonify, request, session, send_file
from jsonschema import Draft202012Validator

MAX_FILE = 5 * 1024 * 1024
ID = re.compile(r"^[A-Za-z0-9]{12}$")
DIRECTIONS = {"north", "south", "east", "west", "northeast", "northwest", "southeast", "southwest", "up", "down"}
STR = {"type": "string"}
STRINGS = {"type": "array", "items": STR}
INTEGER = {"type": "integer", "minimum": 0}
OBJECT = {"type": "object"}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def parse_json(text):
    def pairs(items):
        out = {}
        for key, value in items:
            if key in out:
                raise ValueError(f"Duplicate JSON key: {key}")
            out[key] = value
        return out
    def invalid(value):
        raise ValueError(f"Invalid JSON number: {value}")
    return json.loads(text, object_pairs_hook=pairs, parse_constant=invalid)


def session_signing_key(state):
    """Keep browser sessions valid across editor restarts; key stays private."""
    path = state / "session-signing.key"
    if not path.exists():
        descriptor, temporary = tempfile.mkstemp(prefix=".session-key-", dir=state)
        try:
            with os.fdopen(descriptor, "wb") as output:
                output.write(secrets.token_bytes(32))
                output.flush()
                os.fsync(output.fileno())
            try:
                os.link(temporary, path)  # Install once, without replacing another worker's key.
            except FileExistsError:
                pass
        finally:
            os.unlink(temporary)
    key = path.read_bytes()
    if len(key) != 32:
        raise RuntimeError("Editor session-signing key must contain exactly 32 bytes.")
    return key


def create_app(root, state=None, validator=None, reload_command=None, port=4080, allowed_hosts=None):
    root = Path(root).resolve()
    state = Path(state or root / "var/editor").resolve()
    state.mkdir(parents=True, exist_ok=True)
    app = Flask(__name__, static_folder="static", static_url_path="/static")
    app.secret_key = session_signing_key(state)
    app.config['RELEASE_SAVE_BLOCK'] = os.environ.get('SAVSOUL_EDITOR_SAVE_BLOCK')
    app.config.update(MAX_CONTENT_LENGTH=MAX_FILE + 8192, SESSION_COOKIE_NAME="savsoul_editor",
                      SESSION_COOKIE_HTTPONLY=True, SESSION_COOKIE_SAMESITE="Strict",
                      PERMANENT_SESSION_LIFETIME=dt.timedelta(days=30),
                      SESSION_REFRESH_EACH_REQUEST=True)
    write_lock = threading.RLock()
    catalog_lock = threading.Lock()
    catalog = {"files": None, "until": 0.0}
    world_lock = threading.Lock()
    world_index = {"value": None, "until": 0.0}
    rate_lock = threading.Lock()
    attempts = collections.defaultdict(collections.deque)
    expected_hosts = {f"localhost:{port}", f"127.0.0.1:{port}", f"[::1]:{port}"}
    expected_hosts.update(allowed_hosts or [])
    if app.testing:
        expected_hosts.add("localhost")

    def read_json(path):
        return parse_json(path.read_text(encoding="utf-8-sig"))

    def records(folder):
        # Never return these records through the content API.
        for path in (root / "data" / folder).glob("*.json"):
            if not path.is_symlink():
                try:
                    yield read_json(path)
                except (ValueError, OSError):
                    continue

    def banned(account, character):
        path = root / "data/bans.json"
        bans = read_json(path) if path.exists() else []
        if not isinstance(bans, list):
            return True
        for ban in bans:
            if not isinstance(ban, dict):
                return True
            value = str(ban.get("pattern", "")).lower()
            if ban.get("kind") == "account" and value in {str(account["id"]).lower(), account["identifier"].lower()}:
                return True
            if ban.get("kind") == "character" and value == character["name"].lower():
                return True
            if ban.get("kind") == "ip":
                parts = value.split(".")
                ip = request.remote_addr.split(".")
                if len(parts) <= 4 and len(ip) == 4 and all(p == "*" or p == ip[i] for i, p in enumerate(parts)):
                    return True
        return False

    def wizard(account, char):
        return (char.get("account_id") == account.get("id") and
                char.get("id") in account.get("characters", []) and
                "admin" in char.get("roles", []) and not banned(account, char))

    def authorize():
        if not session.get("account"):
            abort(401, "Log in with a wizard character.")
        account = next((a for a in records("accounts") if a.get("id") == session["account"]), None)
        char = next((c for c in records("characters") if c.get("id") == session.get("character")), None)
        if (not account or not char or not wizard(account, char) or
                not hmac.compare_digest(digest(account.get("password_hash", "").encode()), session.get("credential", ""))):
            session.clear()
            abort(403, "Wizard access is no longer available. Log in again.")
        g.wizard = char["name"]

    def allowed(relative):
        if not isinstance(relative, str) or not relative or "\\" in relative:
            abort(400, "Invalid content path.")
        parts = Path(relative).parts
        if any(p in ("..", ".") or p.startswith(".") for p in parts) or Path(relative).is_absolute():
            abort(403, "Content path is outside the editor.")
        permitted = ((len(parts) >= 3 and parts[0] == "data" and
                      parts[1] in {"areas", "help", "savage-soul", "quests", "socials"}) or
                     (len(parts) == 2 and parts[0] == "assets") or
                     relative == "migration/savsoul-reconstruction.json")
        path = root.joinpath(*parts)
        if not permitted or path.suffix not in {".json", ".md", ".txt"}:
            abort(403, "This file is outside editable game content.")
        if parts[0] == "assets" and path.suffix != ".txt":
            abort(403, "Only text assets are editable.")
        current = root
        for part in parts:
            current = current / part
            if current.is_symlink():
                abort(403, "Symbolic links cannot be edited.")
        if not path.resolve().is_relative_to(root):
            abort(403, "Content path escapes the installation.")
        return path

    def category(rel):
        return {"areas": "World", "help": "Help", "savage-soul": "Rules", "quests": "Quests", "socials": "Socials"}.get(rel.split("/")[1] if rel.startswith("data/") else "", "Assets" if rel.startswith("assets/") else "Migration")

    def activation(rel):
        if rel.startswith("data/help/") or rel in {"assets/motd.txt", "assets/motdlist.txt"}:
            return "Immediate: read on the next request or login."
        if rel.startswith("migration/"):
            return "Migration source: used by the reconstruction tools, not the running world."
        if rel.startswith("assets/"):
            return "Embedded asset: requires a rebuild to change the running game."
        return "Saved blueprint: use Apply world changes to reload the MUD. Active fights and NPC positions reset."

    def area_errors(replacement=None):
        areas = []
        for file in (root / "data/areas").glob("*.json"):
            if file.is_symlink():
                continue
            if replacement and file == replacement[0]:
                value = replacement[1]
            else:
                value = read_json(file)
            if value.get("canonical", False):
                areas.append(value)
        if replacement and not replacement[0].exists() and replacement[1].get("canonical", False):
            areas.append(replacement[1])
        errors, rooms, identities, slugs = set(), {}, set(), set()
        for area in areas:
            if area["id"] in identities:
                errors.add(f"Duplicate area ID {area['id']}")
            identities.add(area["id"])
            if area["slug"] in slugs:
                errors.add(f"Duplicate area slug {area['slug']}")
            slugs.add(area["slug"])
            room_slugs = set()
            for room in area.get("rooms", []):
                if room["id"] in rooms:
                    errors.add(f"Duplicate room ID {room['id']}")
                rooms[room["id"]] = room
                if room["slug"] in room_slugs:
                    errors.add(f"Duplicate room slug {area['slug']}:{room['slug']}")
                room_slugs.add(room["slug"])
        for area in areas:
            if area.get("starting_room") and area["starting_room"] not in rooms:
                errors.add(f"Starting room {area['starting_room']} does not exist")
            for room in area.get("rooms", []):
                for direction, target in room.get("exits", {}).items():
                    if direction not in DIRECTIONS:
                        errors.add(f"{room['slug']}: unknown direction {direction}")
                    if target not in rooms:
                        errors.add(f"{room['slug']}: exit {direction} points to missing room {target}")
                for direction in room.get("doors", {}):
                    if direction not in room.get("exits", {}):
                        errors.add(f"{room['slug']}: door {direction} has no exit")
        return errors

    def validate(rel, text, cross_check=True):
        if len(text.encode("utf-8")) > MAX_FILE or "\x00" in text:
            abort(400, "Content is too large or contains a null character.")
        if not rel.endswith(".json"):
            return []
        value = parse_json(text)
        if rel.startswith("data/areas/"):
            if not isinstance(value, dict):
                raise ValueError("An area must be a JSON object.")
            if not validator:
                raise ValueError("The MUD blueprint validator is unavailable; refusing to save an area.")
            with tempfile.NamedTemporaryFile(mode="w", suffix=".json", encoding="utf-8") as tmp:
                tmp.write(text); tmp.flush()
                result = subprocess.run([str(validator), "--validate-area", tmp.name], capture_output=True, text=True, timeout=15)
            if result.returncode:
                raise ValueError(result.stderr.strip() or "MUD rejected this area blueprint.")
            if not cross_check:
                return []
            baseline = area_errors()
            updated = area_errors((allowed(rel), value))
            introduced = updated - baseline
            if introduced:
                raise ValueError("\n".join(sorted(introduced)[:20]))
            return sorted(updated)
        if rel in ("data/savage-soul/races.json", "data/savage-soul/creator.json"):
            if not validator:
                raise ValueError("The MUD race validator is unavailable; refusing to save race templates.")
            with tempfile.NamedTemporaryFile(mode="w", suffix=".json", encoding="utf-8") as tmp:
                tmp.write(text); tmp.flush()
                result = subprocess.run([str(validator), "--validate-creator" if rel.endswith("creator.json") else "--validate-races", tmp.name], capture_output=True, text=True, timeout=15)
            if result.returncode:
                raise ValueError(result.stderr.strip() or "MUD rejected these race templates.")
            return []
        schema = None
        if rel == "data/help/index.json":
            schema = {"type": "array", "items": {"type": "object", "required": ["id", "title", "category", "file"], "properties": {"id": STR, "title": STR, "category": STR, "file": STR, "aliases": STRINGS, "wizard": {"type": "boolean"}}}}
        elif rel == "data/savage-soul/world.json":
            schema = {"type": "object", "properties": {"corpse_decay_seconds": {"type": "integer", "minimum": 1, "maximum": 31536000}}}
        elif rel == "data/savage-soul/population.json":
            movement = {"type": "object", "properties": {"mode": {"enum": ["stay", "wander", "guard_room", "guard_npc"]}, "interval_seconds": INTEGER, "max_distance": {"type": ["integer", "null"], "minimum": 0, "maximum": 65535}, "same_area": {"type": "boolean"}, "target": {"type": ["string", "null"]}}}
            spawn = {"type": "object", "properties": {"max_alive": {"type": "integer", "minimum": 0, "maximum": 65535}, "respawn_seconds": INTEGER, "pool": {"type": ["string", "null"]}, "max_total": {"type": ["integer", "null"], "minimum": 0, "maximum": 65535}, "movement": movement}}
            schema = {"type": "object", "properties": {"profiles": {"type": "object", "additionalProperties": spawn}}}
        elif rel == "data/savage-soul/items.json":
            schema = {"type": "array", "items": {"type": "object", "required": ["name"], "properties": {"name": STR, "keywords": STRINGS, "price": INTEGER, "sell_price": INTEGER, "weight_kg": {"type": "number", "minimum": 0}}}}
        elif rel.startswith("data/socials/") or rel.endswith("npc-profiles.json"):
            schema = OBJECT
        if schema:
            error = next(iter(Draft202012Validator(schema).iter_errors(value)), None)
            if error:
                raise ValueError(f"{'.'.join(map(str, error.path)) or 'document'}: {error.message}")
        if rel.endswith("npc-profiles.json"):
            if not validator:
                raise ValueError("Native NPC validator is unavailable.")
            npcs = []
            for name, profile in value.items():
                if not isinstance(profile, dict):
                    raise ValueError(f"NPC profile {name} must be an object.")
                npcs.append({"name": name, "description": [], **profile})
            area = {"id": "Validate0001", "slug": "validation", "name": "Validation", "rooms": [
                {"id": "Validate0002", "slug": "validation", "name": "Validation", "description": "", "npcs": npcs}]}
            with tempfile.NamedTemporaryFile(mode="w", suffix=".json", encoding="utf-8") as tmp:
                json.dump(area, tmp); tmp.flush()
                result = subprocess.run([str(validator), "--validate-area", tmp.name], capture_output=True, text=True, timeout=15)
            if result.returncode:
                raise ValueError(result.stderr.strip() or "MUD rejected the NPC profiles.")
        if rel == "data/help/index.json":
            ids = set()
            for topic in value:
                if topic["id"] in ids:
                    raise ValueError(f"Duplicate help ID: {topic['id']}")
                ids.add(topic["id"])
                file = allowed("data/help/" + topic["file"])
                if not file.is_file():
                    raise ValueError(f"Help file does not exist: {topic['file']}")
        return []

    def snapshot(rel):
        path = allowed(rel)
        if not path.is_file():
            abort(404, "Content file not found.")
        if path.stat().st_size > MAX_FILE:
            abort(413, "Content file is too large.")
        data = path.read_bytes()
        return {"path": rel, "text": data.decode("utf-8-sig"), "revision": digest(data), "activation": activation(rel)}

    def audit(action, **details):
        record = {"at": dt.datetime.now(dt.timezone.utc).isoformat(), "wizard": g.get("wizard", ""), "action": action, **details}
        with (state / "audit.jsonl").open("a", encoding="utf-8") as log:
            log.write(json.dumps(record) + "\n")

    def check_save_block():
        block = app.config.get('RELEASE_SAVE_BLOCK')
        if block and Path(block).exists():
            abort(503, "A server release is in progress. Your draft has not been saved; try again shortly.")

    def commit(rel, text, revision):
        path = allowed(rel)
        with write_lock:
            check_save_block()
            old = path.read_bytes() if path.exists() else None
            expected = digest(old) if old is not None else "new"
            if not isinstance(revision, str) or not hmac.compare_digest(expected, revision):
                abort(409, "File changed since you opened it. Reload before saving; your draft has not been overwritten.")
            warnings = validate(rel, text)
            new = text.encode("utf-8")
            # Backups and audit must succeed before the active content is replaced.
            folder = state / "backups" / digest(rel.encode())
            folder.mkdir(parents=True, exist_ok=True)
            version = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%S") + "-" + secrets.token_hex(6)
            if old is not None:
                with (folder / (version + ".content")).open("wb") as backup:
                    backup.write(old); backup.flush(); os.fsync(backup.fileno())
                (folder / (version + ".json")).write_text(json.dumps({"id": version, "path": rel, "wizard": g.wizard, "at": dt.datetime.now(dt.timezone.utc).isoformat(), "revision": expected}), encoding="utf-8")
            audit("save", path=rel, before=expected, after=digest(new))
            path.parent.mkdir(parents=True, exist_ok=True)
            temp = None
            try:
                with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".editor-", delete=False) as handle:
                    temp = handle.name
                    handle.write(new); handle.flush(); os.fsync(handle.fileno())
                check_save_block()
                os.replace(temp, path)
                temp = None
                with catalog_lock:
                    catalog["until"] = 0.0
                with world_lock:
                    world_index["until"] = 0.0
            finally:
                if temp:
                    Path(temp).unlink(missing_ok=True)
            return {**snapshot(rel), "warnings": warnings}

    @app.before_request
    def guard():
        if request.host not in expected_hosts:
            abort(403, "This editor address is not allowed.")
        if request.path.startswith("/api/"):
            if request.path not in {"/api/session", "/api/login"}:
                authorize()
            if request.method not in {"GET", "HEAD", "OPTIONS"}:
                if request.headers.get("Origin") and request.headers["Origin"] != request.host_url.rstrip("/"):
                    abort(403, "Cross-origin changes are not allowed.")
                if not hmac.compare_digest(request.headers.get("X-CSRF-Token", ""), session.get("csrf", "!")):
                    abort(403, "Session token missing. Reload the page.")

    @app.after_request
    def headers(response):
        response.headers.update({"Cache-Control": "no-store", "X-Content-Type-Options": "nosniff", "X-Frame-Options": "DENY", "Referrer-Policy": "same-origin", "Content-Security-Policy": "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"})
        return response

    @app.errorhandler(Exception)
    def error(exc):
        from werkzeug.exceptions import HTTPException
        if isinstance(exc, HTTPException):
            return jsonify(error=exc.description), exc.code
        if isinstance(exc, (ValueError, UnicodeError)):
            return jsonify(error=str(exc)), 400
        app.logger.exception("Editor request failed")
        return jsonify(error="The operation failed. No successful save was reported; check the editor service log."), 500

    @app.get("/")
    def index():
        return app.send_static_file("index.html")

    @app.get("/media/protocol-ready.wav")
    def protocol_sample():
        media = root / "assets" / "media"
        sample = media / "protocol-ready.wav"
        if media.is_symlink() or sample.is_symlink() or not sample.is_file():
            abort(404)
        return send_file(sample, mimetype="audio/wav", max_age=86400)

    @app.get("/api/session")
    def who():
        session.setdefault("csrf", secrets.token_urlsafe(32))
        if session.get("account"):
            authorize()
        return jsonify(authenticated=bool(session.get("account")), wizard=g.get("wizard"), csrf=session["csrf"])

    @app.post("/api/login")
    def login():
        with rate_lock:
            queue = attempts[request.remote_addr]
            while queue and queue[0] < time.monotonic() - 60:
                queue.popleft()
            if len(queue) >= 10:
                abort(429, "Too many login attempts. Wait a minute.")
            queue.append(time.monotonic())
        data = request.get_json()
        identifier = str(data.get("identifier", "")).strip().lower()
        password = str(data.get("password", "")).strip()
        chars = list(records("characters"))
        named = next((c for c in chars if c.get("name", "").lower() == identifier), None)
        account = next((a for a in records("accounts") if a.get("identifier", "").lower() == identifier or (named and a.get("id") == named.get("account_id"))), None)
        candidate = next((c for c in chars if account and wizard(account, c) and (not named or c["id"] == named["id"])), None)
        supplied = hashlib.sha256(password.encode()).hexdigest()
        if not account or not hmac.compare_digest(supplied, account.get("password_hash", "0" * 64)) or not candidate:
            abort(401, "Invalid MUD credentials or this character is not a wizard.")
        session.clear(); session.permanent = True
        session.update(account=account["id"], character=candidate["id"], credential=digest(account["password_hash"].encode()), csrf=secrets.token_urlsafe(32))
        g.wizard = candidate["name"]
        audit("login")
        return jsonify(wizard=g.wizard, csrf=session["csrf"])

    @app.post("/api/logout")
    def logout():
        session.clear()
        return jsonify(ok=True)

    def scan_catalog():
        result = []
        def walk(directory, relative, nested=True):
            try:
                with os.scandir(directory) as entries:
                    for entry in entries:
                        if entry.name.startswith(".") or entry.is_symlink():
                            continue
                        rel = relative + "/" + entry.name
                        if nested and entry.is_dir(follow_symlinks=False):
                            walk(entry.path, rel)
                        elif entry.is_file(follow_symlinks=False):
                            suffix = Path(entry.name).suffix
                            if suffix in ({".txt"} if relative == "assets" else {".json", ".md", ".txt"}):
                                result.append({"path": rel, "category": category(rel)})
            except FileNotFoundError:
                pass
        for directory in ["data/areas", "data/help", "data/savage-soul", "data/quests", "data/socials", "assets"]:
            # Check the fixed roots once, not every ancestor of every file.
            # Reads/writes still call allowed() afresh, including all link checks.
            current = root
            for part in directory.split("/"):
                current = current / part
                if current.is_symlink():
                    abort(403, "Symbolic content directories cannot be listed.")
            walk(current, directory, nested=directory != "assets")
        migration = root / "migration"
        if not migration.is_symlink():
            try:
                with os.scandir(migration) as entries:
                    for entry in entries:
                        if entry.name == "savsoul-reconstruction.json" and not entry.is_symlink() and entry.is_file(follow_symlinks=False):
                            result.append({"path": "migration/" + entry.name, "category": "Migration"})
            except FileNotFoundError:
                pass
        return sorted(result, key=lambda item: item["path"])

    @app.get("/api/files")
    def files():
        # Directory-entry types are cached by scandir; do not stat/resolve each
        # file on the network-backed content drive just to display its name.
        with catalog_lock:
            if catalog["files"] is None or time.monotonic() >= catalog["until"]:
                catalog["files"] = scan_catalog()
                catalog["until"] = time.monotonic() + 5.0
            result = catalog["files"]
        return jsonify(files=result)

    @app.get("/api/world")
    def world():
        # One bounded read per area, no per-room filesystem metadata calls.
        with world_lock:
            if world_index["value"] is None or time.monotonic() >= world_index["until"]:
                allowed("data/areas/__map__.json")
                areas, errors = [], []
                with os.scandir(root / "data/areas") as entries:
                    for entry in sorted(entries, key=lambda e: e.name):
                        if entry.name.startswith(".") or not entry.name.endswith(".json") or entry.is_symlink() or not entry.is_file(follow_symlinks=False):
                            continue
                        path = "data/areas/" + entry.name
                        try:
                            with open(entry.path, "r", encoding="utf-8-sig") as handle:
                                text = handle.read(MAX_FILE + 1)
                            if len(text.encode("utf-8")) > MAX_FILE:
                                raise ValueError("Area exceeds editor size limit")
                            area = parse_json(text)
                            if not isinstance(area, dict) or not isinstance(area.get("rooms", []), list):
                                raise ValueError("Area rooms must be an array")
                            rooms = []
                            for room in area.get("rooms", []):
                                if not isinstance(room, dict):
                                    raise ValueError("Room must be an object")
                                record = {key: room.get(key) for key in ["id", "slug", "name", "description", "exits", "doors"]}
                                for key in ["npcs", "objects", "scenery"]:
                                    record[key] = [{"name": item.get("name", "")} for item in room.get(key, []) if isinstance(item, dict)]
                                rooms.append(record)
                            areas.append({**{key: area.get(key) for key in ["id", "slug", "name", "canonical", "starting_room"]}, "path": path, "rooms": rooms})
                        except (ValueError, OSError, TypeError) as error:
                            errors.append({"path": path, "error": str(error)})
                world_index["value"] = {"areas": areas, "errors": errors}
                world_index["until"] = time.monotonic() + 5.0
            result = world_index["value"]
        return jsonify(result)

    @app.get("/api/file")
    def file():
        return jsonify(snapshot(request.args.get("path")))

    @app.post("/api/save")
    def save():
        data = request.get_json()
        if not isinstance(data.get("text"), str):
            abort(400, "Content must be text.")
        return jsonify(commit(data.get("path"), data["text"], data.get("revision")))

    @app.post("/api/validate")
    def check():
        data = request.get_json(); allowed(data.get("path"))
        if not isinstance(data.get("text"), str):
            abort(400, "Content must be text.")
        return jsonify(ok=True, warnings=validate(data["path"], data["text"]))

    @app.get("/api/history")
    def history():
        rel = request.args.get("path"); allowed(rel)
        folder = state / "backups" / digest(rel.encode())
        return jsonify(versions=[read_json(file) for file in sorted(folder.glob("*.json"), reverse=True)[:100]])

    @app.post("/api/restore")
    def restore():
        data = request.get_json(); rel = data.get("path"); allowed(rel)
        version = str(data.get("version", ""))
        if not re.fullmatch(r"[0-9]{8}T[0-9]{6}-[0-9a-f]{12}", version):
            abort(400, "Invalid backup version.")
        path = state / "backups" / digest(rel.encode()) / (version + ".content")
        if not path.is_file():
            abort(404, "Backup not found.")
        return jsonify(commit(rel, path.read_text(encoding="utf-8-sig"), data.get("revision")))

    @app.post("/api/apply")
    def apply():
        if not reload_command:
            abort(503, "World reload is not configured.")
        with write_lock:
            for file in (root / "data/areas").glob("*.json"):
                validate(file.relative_to(root).as_posix(), file.read_text(encoding="utf-8-sig"), cross_check=False)
            for directory in ["data/help", "data/savage-soul", "data/quests", "data/socials"]:
                for file in (root / directory).rglob("*.json"):
                    allowed(file.relative_to(root).as_posix())
                    validate(file.relative_to(root).as_posix(), file.read_text(encoding="utf-8-sig"))
            errors = area_errors()
            if errors:
                abort(400, "World has unresolved links: " + "; ".join(sorted(errors)[:10]))
            audit("apply-world")
            result = subprocess.run(reload_command, capture_output=True, text=True, timeout=10)
            if result.returncode:
                abort(503, "MUD reload could not be requested. Saved content remains on disk.")
        return jsonify(ok=True, message="World reload requested. Connections are preserved; active fights and NPC positions reset.")

    return app


if __name__ == "__main__":
    from waitress import serve
    base = Path(os.environ.get("GRIM_ROOT", Path(__file__).resolve().parents[2]))
    application = create_app(base, os.environ.get("GRIM_EDITOR_STATE"),
                             os.environ.get("GRIM_VALIDATOR", "/tmp/savsoul-grim-runtime/savsoul"),
                             ["systemctl", "--user", "kill", "--kill-whom=main", "--signal=SIGUSR2", "savsoul-grim"])
    serve(application, host="127.0.0.1", port=4080, threads=4, max_request_body_size=MAX_FILE + 8192)
