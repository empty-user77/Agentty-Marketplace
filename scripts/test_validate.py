#!/usr/bin/env python3
"""What validate.py must refuse. Run with `python3 -m unittest discover -s scripts`."""

import json
import tempfile
import unittest
from pathlib import Path

import validate


def entry(**changes) -> dict:
    base = {
        "id": "hello-world",
        "name": "Hello World",
        "version": "0.1.0",
        "publisher": "Someone",
        "description": "A panel that says hello.",
        "license": "MIT",
        "source": "https://github.com/someone/agentty-hello-world",
        "surface": "sidebar",
        "permissions": [],
        "module": {
            "url": "https://github.com/someone/agentty-hello-world/releases/download/v0.1.0/hello-world.wasm",
            "sha256": "a" * 64,
            "size": 93292,
        },
    }
    base.update(changes)
    return base


def check(data: dict, name: str = "hello-world.json"):
    with tempfile.TemporaryDirectory() as folder:
        path = Path(folder) / name
        path.write_text(json.dumps(data), encoding="utf-8")
        return validate.check(path)


class Entries(unittest.TestCase):
    def test_a_good_entry_passes(self):
        self.assertEqual(check(entry())["id"], "hello-world")

    def test_the_file_name_is_the_id(self):
        with self.assertRaises(validate.Problem):
            check(entry(), name="something-else.json")

    def test_ids_and_versions_have_a_shape(self):
        for bad in [entry(id="Hello"), entry(id="a"), entry(id="has space"), entry(version="1.0"), entry(version="latest")]:
            with self.assertRaises(validate.Problem):
                check(bad)

    def test_the_source_must_be_somewhere_anyone_can_read(self):
        for bad in ["http://github.com/a/b", "https://example.com/a/b", "https://github.example.com/a/b", ""]:
            with self.assertRaises(validate.Problem):
                check(entry(source=bad))

    def test_the_module_comes_from_a_release_and_names_its_version(self):
        module = dict(entry()["module"])
        for url in [
            "http://github.com/a/b/releases/download/v0.1.0/x.wasm",   # not https
            "https://example.com/x-0.1.0.wasm",                        # not a release host
            "https://github.com/a/b/releases/download/latest/x.wasm",  # no version: can be swapped
        ]:
            with self.assertRaises(validate.Problem):
                check(entry(module={**module, "url": url}))

    def test_the_checksum_and_size_are_real(self):
        module = dict(entry()["module"])
        for change in [{"sha256": "nope"}, {"sha256": "A" * 64}, {"size": 0}, {"size": 9 * 1024 * 1024}, {"size": "93292"}]:
            with self.assertRaises(validate.Problem):
                check(entry(module={**module, **change}))

    def test_permissions_are_ones_agentty_can_explain(self):
        with self.assertRaises(validate.Problem):
            check(entry(permissions=["fs.everything"]))
        with self.assertRaises(validate.Problem):
            check(entry(permissions=["net.request", "net.request"]))
        self.assertEqual(check(entry(permissions=["net.request"]))["permissions"], ["net.request"])

    def test_reading_the_users_work_and_sending_requests_is_explained(self):
        both = ["net.request", "session.read"]
        with self.assertRaises(validate.Problem):
            check(entry(permissions=both, description="Sends things."))
        # A description that says what it does with them passes.
        said = "Reads the session and posts a summary to the endpoint you configure (net.request)."
        self.assertEqual(len(check(entry(permissions=both, description=said))["permissions"]), 2)

    def test_text_is_bounded_and_plain(self):
        for bad in [entry(name="x" * 61), entry(description="x" * 301), entry(name="two\nlines"), entry(publisher="")]:
            with self.assertRaises(validate.Problem):
                check(bad)

    def test_an_entry_says_which_protocol_it_is_built_against(self):
        # Left out: the protocol that existed before the field did.
        self.assertEqual(check(entry()).get("apiVersion", 1), 1)
        self.assertEqual(check(entry(apiVersion=1))["apiVersion"], 1)

    def test_a_protocol_agentty_does_not_speak_yet_is_refused(self):
        for bad in [validate.PLUGIN_API_VERSION + 1, 0, -1, "1", 1.0, True, None]:
            with self.assertRaises(validate.Problem):
                check(entry(apiVersion=bad))

    def test_surfaces_and_modes_are_ones_agentty_has(self):
        with self.assertRaises(validate.Problem):
            check(entry(surface="everywhere"))
        with self.assertRaises(validate.Problem):
            check(entry(mode="sideways"))


if __name__ == "__main__":
    unittest.main()
