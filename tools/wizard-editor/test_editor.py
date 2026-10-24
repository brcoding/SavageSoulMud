import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("editor", Path(__file__).with_name("server.py"))
editor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(editor)

class EditorTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for folder in ["data/accounts", "data/characters", "data/areas", "data/help", "assets"]:
            (self.root / folder).mkdir(parents=True)
        self.account = {"id": "Account00001", "identifier": "wizard@example.com", "password_hash": hashlib.sha256(b"secretpw").hexdigest(), "characters": ["Wizard000001"]}
        self.char = {"id": "Wizard000001", "account_id": self.account["id"], "name": "Wizard", "roles": ["admin"]}
        self.write("data/accounts/a.json", self.account)
        self.write("data/characters/Wizard.json", self.char)
        (self.root / "assets/motdlist.txt").write_text("original\n")
        self.app = editor.create_app(self.root, self.root / "state", os.environ.get("GRIM_TEST_VALIDATOR"))
        self.app.testing = True
        self.client = self.app.test_client()
        self.csrf = self.get("session").json["csrf"]
    def write(self, path, value):
        (self.root / path).write_text(json.dumps(value))
    def get(self, path):
        return self.client.get("/api/" + path, base_url="http://localhost:4080")
    def post(self, path, body, token=True):
        return self.client.post("/api/" + path, json=body, headers={"X-CSRF-Token": self.csrf} if token else {}, base_url="http://localhost:4080")
    def login(self, identifier="Wizard"):
        result = self.post("login", {"identifier": identifier, "password": "secretpw"})
        self.assertEqual(result.status_code, 200, result.json)
        self.csrf = result.json["csrf"]
    def test_public_https_proxy_keeps_origin_csrf_wizard_auth_and_secure_cookie(self):
        from werkzeug.middleware.proxy_fix import ProxyFix
        app = editor.create_app(self.root, self.root / "state", allowed_hosts=["savsoul.com:4081"])
        app.config["SESSION_COOKIE_SECURE"] = True
        app.wsgi_app = ProxyFix(app.wsgi_app, x_for=1, x_proto=1)
        client = app.test_client()
        base = "http://savsoul.com:4081"
        forwarded = {"X-Forwarded-Proto": "https", "X-Forwarded-For": "192.0.2.10"}
        response = client.get("/api/session", base_url=base, headers=forwarded)
        self.assertEqual(response.status_code, 200)
        self.assertIn("Secure", response.headers["Set-Cookie"])
        token = response.json["csrf"]
        self.assertEqual(client.get("/api/files", base_url=base, headers=forwarded).status_code, 401)
        headers = {**forwarded, "Origin": "https://savsoul.com:4081", "X-CSRF-Token": token}
        bad = client.post("/api/login", base_url=base, headers={**headers, "Origin": "https://evil.example"}, json={})
        self.assertEqual(bad.status_code, 403)
        result = client.post("/api/login", base_url=base, headers=headers, json={"identifier": "Wizard", "password": "secretpw"})
        self.assertEqual(result.status_code, 200, result.json)
        self.assertEqual(result.json["wizard"], "Wizard")
        self.assertEqual(client.get("/api/files", base_url=base, headers=forwarded).status_code, 200)
        self.assertEqual(client.get("/", base_url="http://evil.example", headers=forwarded).status_code, 403)

    def test_remembered_cookie_survives_editor_restart_and_logout_clears_it(self):
        self.login()
        response = self.get("session")
        header = response.headers.get("Set-Cookie", "")
        self.assertIn("Expires=", header)
        self.assertIn("HttpOnly", header)
        self.assertIn("SameSite=Strict", header)
        self.assertGreaterEqual(self.app.permanent_session_lifetime.total_seconds(), 3 * 86400)
        cookie = self.client.get_cookie("savsoul_editor", domain="localhost")
        restarted = editor.create_app(self.root, self.root / "state")
        fresh_browser = restarted.test_client()
        fresh_browser.set_cookie("savsoul_editor", cookie.value, domain="localhost")
        result = fresh_browser.get("/api/session", base_url="http://localhost:4080")
        self.assertTrue(result.json["authenticated"])
        self.assertEqual(result.json["wizard"], "Wizard")
        self.assertEqual(self.post("logout", {}).status_code, 200)
        self.assertFalse(self.get("session").json["authenticated"])
        # Long-lived cookies still recheck the actual wizard role on every request.
        self.char["roles"] = []; self.write("data/characters/Wizard.json", self.char)
        self.assertEqual(fresh_browser.get("/api/files", base_url="http://localhost:4080").status_code, 403)

    def test_login_is_required_and_regular_characters_denied(self):
        self.assertEqual(self.get("files").status_code, 401)
        self.assertEqual(self.post("login", {"identifier": "Wizard", "password": "wrong"}).status_code, 401)
        self.char["roles"] = []; self.write("data/characters/Wizard.json", self.char)
        self.assertEqual(self.post("login", {"identifier": "wizard@example.com", "password": "secretpw"}).status_code, 401)
    def test_wizard_ownership_revocation_and_bans(self):
        self.login("wizard@example.com")
        self.assertEqual(self.get("files").status_code, 200)
        self.char["roles"] = []; self.write("data/characters/Wizard.json", self.char)
        self.assertEqual(self.get("files").status_code, 403)
        self.csrf = self.get("session").json["csrf"]
        self.char["roles"] = ["admin"]; self.char["account_id"] = "other"; self.write("data/characters/Wizard.json", self.char)
        self.assertEqual(self.post("login", {"identifier": "Wizard", "password": "secretpw"}).status_code, 401)
        self.char["account_id"] = self.account["id"]; self.write("data/characters/Wizard.json", self.char)
        self.csrf = self.get("session").json["csrf"]
        self.login()
        self.write("data/bans.json", [{"kind": "character", "pattern": "wizard"}])
        self.assertEqual(self.get("files").status_code, 403)
    def test_csrf_paths_symlinks_and_host(self):
        self.login()
        self.assertEqual(self.post("save", {}, token=False).status_code, 403)
        for path in ["data/accounts/a.json", "data/characters/Wizard.json", "assets/../../data/accounts/a.json", "/etc/passwd"]:
            self.assertEqual(self.get("file?path=" + path).status_code, 403)
        (self.root / "assets/escape.txt").symlink_to(self.root / "data/accounts/a.json")
        self.assertEqual(self.get("file?path=assets/escape.txt").status_code, 403)
        self.assertEqual(self.client.get("/", base_url="http://evil.example").status_code, 403)
    def test_release_blocks_saves_until_transaction_finishes(self):
        self.login()
        loaded = self.get('file?path=assets/motdlist.txt').json
        block = self.root / 'editor-save-block'
        self.app.config['RELEASE_SAVE_BLOCK'] = str(block)
        block.touch()
        payload = {'path': 'assets/motdlist.txt', 'revision': loaded['revision'], 'text': 'changed\n'}
        self.assertEqual(self.post('save', payload).status_code, 503)
        self.assertEqual((self.root / 'assets/motdlist.txt').read_text(), 'original\n')
        block.unlink()
        self.assertEqual(self.post('save', payload).status_code, 200)

    def test_save_conflict_backup_and_restore(self):
        self.login()
        loaded = self.get("file?path=assets/motdlist.txt").json
        result = self.post("save", {"path": loaded["path"], "revision": loaded["revision"], "text": "updated\n"})
        self.assertEqual(result.status_code, 200, result.json)
        self.assertEqual(self.post("save", {**loaded, "text": "stale"}).status_code, 409)
        versions = self.get("history?path=assets/motdlist.txt").json["versions"]
        result = self.post("restore", {"path": loaded["path"], "revision": result.json["revision"], "version": versions[0]["id"]})
        self.assertEqual(result.status_code, 200, result.json)
        self.assertEqual((self.root / loaded["path"]).read_text(), "original\n")
    def test_world_index_is_private_bounded_and_skips_links(self):
        self.assertEqual(self.get("world").status_code, 401)
        self.login()
        area = {"id": "Area00000001", "name": "Test", "slug": "test", "canonical": True, "rooms": [{"id": "Room00000001", "name": "Room", "slug": "room", "description": "A room", "exits": {}, "npcs": [{"name": "NPC", "soul": {"private-field": 1}}]}]}
        self.write("data/areas/test.json", area)
        (self.root / "data/areas/link.json").symlink_to(self.root / "data/accounts/a.json")
        (self.root / "data/areas/broken.json").write_text("{bad")
        result = self.get("world")
        self.assertEqual(result.status_code, 200, result.json)
        self.assertEqual(len(result.json["areas"]), 1)
        self.assertEqual(result.json["areas"][0]["rooms"][0]["npcs"], [{"name": "NPC"}])
        self.assertEqual(result.json["errors"][0]["path"], "data/areas/broken.json")

    def test_catalog_skips_links_and_refreshes_after_save(self):
        self.login()
        (self.root / "assets/secret.txt").symlink_to(self.root / "data/accounts/a.json")
        (self.root / "data/help/escape").symlink_to(self.root / "data/accounts", target_is_directory=True)
        listed = self.get("files").json["files"]
        paths = {f["path"] for f in listed}
        self.assertIn("assets/motdlist.txt", paths)
        self.assertNotIn("assets/secret.txt", paths)
        self.assertFalse(any("escape" in p for p in paths))
        result = self.post("save", {"path": "assets/new-message.txt", "revision": "new", "text": "new message"})
        self.assertEqual(result.status_code, 200, result.json)
        self.assertIn("assets/new-message.txt", {f["path"] for f in self.get("files").json["files"]})

    def test_invalid_json_and_world_settings_not_written(self):
        self.login()
        for text in ['{"corpse_decay_seconds":-1}', '{"x":1,"x":2}', '{"x":NaN}', '{bad']:
            response = self.post("save", {"path": "data/savage-soul/world.json", "revision": "new", "text": text})
            self.assertEqual(response.status_code, 400, response.json)
        self.assertFalse((self.root / "data/savage-soul/world.json").exists())
    @unittest.skipUnless(os.environ.get("GRIM_TEST_VALIDATOR"), "requires native validator")
    def test_native_blueprint_validation_and_exit_links(self):
        self.login()
        area = {"id": "Area00000001", "slug": "test", "name": "Test", "canonical": True, "rooms": [{"id": "Room00000001", "slug": "one", "name": "One", "description": "A room.", "exits": {}}]}
        def save():
            return self.post("save", {"path": "data/areas/test.json", "revision": "new", "text": json.dumps(area)})
        area["rooms"][0]["npcs"] = [{"name": "Invalid", "description": 12}]
        self.assertEqual(save().status_code, 400)
        del area["rooms"][0]["npcs"]
        area["rooms"][0]["exits"] = {"south": "Missing00001"}
        self.assertEqual(save().status_code, 400)
        area["rooms"][0]["exits"] = {}
        self.assertEqual(save().status_code, 200)

if __name__ == "__main__":
    unittest.main(verbosity=2)
