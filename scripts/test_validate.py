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
        "build": {
            "repository": "https://github.com/someone/agentty-hello-world",
            "rev": "b" * 40,
            "path": ".",
            "toolchain": validate.BUILD_TOOLCHAIN,
            "artifact": "target/wasm32-unknown-unknown/release/hello_world.wasm",
        },
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


class HowTheModuleWasBuilt(unittest.TestCase):
    """An entry has to say where its module comes from, in enough detail to build it again."""

    def build(self, **changes) -> dict:
        return {**entry()["build"], **changes}

    def test_an_entry_without_a_build_block_is_refused(self):
        without = entry()
        del without["build"]
        with self.assertRaises(validate.Problem):
            check(without)

    def test_every_part_of_it_is_required(self):
        for field in ("repository", "rev", "path", "toolchain", "artifact"):
            missing = self.build()
            del missing[field]
            with self.assertRaises(validate.Problem):
                check(entry(build=missing))

    def test_the_commit_is_a_commit(self):
        # A tag or a branch can be moved to other code the day after the review.
        for bad in ["v0.1.0", "main", "b" * 39, "b" * 41, "B" * 40, "g" * 40, "", None, 1]:
            with self.assertRaises(validate.Problem):
                check(entry(build=self.build(rev=bad)))

    def test_the_source_and_the_build_are_the_same_repository(self):
        # Otherwise the entry links to code anyone can read, and ships a module built from
        # somewhere else entirely.
        with self.assertRaises(validate.Problem):
            check(entry(build=self.build(repository="https://github.com/someone/something-else")))
        with self.assertRaises(validate.Problem):
            check(entry(build=self.build(repository="https://gitlab.com/someone/agentty-hello-world")))
        # The same repository written differently is still the same repository.
        same = entry(build=self.build(repository="https://github.com/Someone/agentty-hello-world.git"))
        self.assertEqual(check(same)["id"], "hello-world")

    def test_the_build_repository_is_somewhere_anyone_can_read(self):
        for bad in ["http://github.com/a/b", "https://example.com/a/b", "https://github.com/a", ""]:
            with self.assertRaises(validate.Problem):
                check(entry(build=self.build(repository=bad), source=bad))

    def test_paths_stay_inside_the_checkout(self):
        for bad in ["/etc", "../outside", "src/../../outside", "src/plugin/..", "a b", "$(whoami)", "x;rm -rf /"]:
            with self.assertRaises(validate.Problem):
                check(entry(build=self.build(path=bad)))
        for bad in ["/etc/passwd.wasm", "../outside.wasm", "a/../../out.wasm", "a b.wasm", "$(whoami).wasm"]:
            with self.assertRaises(validate.Problem):
                check(entry(build=self.build(artifact=bad)))

    def test_the_artifact_is_a_wasm_module(self):
        with self.assertRaises(validate.Problem):
            check(entry(build=self.build(artifact="target/release/hello_world.so")))

    def test_the_toolchain_is_the_one_the_marketplace_builds_with(self):
        # Anything else and the rebuild would not reproduce, so there would be nothing to compare.
        for bad in ["1.98", "stable", "nightly", "0.0.1", ""]:
            with self.assertRaises(validate.Problem):
                check(entry(build=self.build(toolchain=bad)))


class TheIndex(unittest.TestCase):
    """index.json is rebuilt by CI and compared to what is committed, so it has to come out the
    same every time. A timestamp that moved on every run made that comparison fail forever."""

    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.index = Path(self.folder.name) / "index.json"
        original = validate.INDEX
        validate.INDEX = self.index
        self.addCleanup(setattr, validate, "INDEX", original)

    def test_writing_it_twice_over_the_same_entries_changes_nothing(self):
        validate.write_index([entry()])
        first = self.index.read_text()
        self.assertTrue(validate.write_index([entry()]) is False)
        self.assertEqual(first, self.index.read_text())

    def test_it_is_dated_again_when_the_list_actually_changes(self):
        validate.write_index([entry()])
        # Back-dated, because two writes a millisecond apart share a timestamp to the second.
        dated = json.loads(self.index.read_text())
        dated["updated"] = "2020-01-01T00:00:00Z"
        self.index.write_text(json.dumps(dated))

        # The same list again: the date it was last really changed is kept.
        self.assertFalse(validate.write_index([entry()]))
        self.assertEqual(json.loads(self.index.read_text())["updated"], "2020-01-01T00:00:00Z")

        # A different list: dated again.
        self.assertTrue(validate.write_index([entry(), entry(id="second")]))
        after = json.loads(self.index.read_text())
        self.assertNotEqual(after["updated"], "2020-01-01T00:00:00Z")
        self.assertEqual(len(after["plugins"]), 2)

    def test_a_missing_or_broken_index_is_simply_written(self):
        self.index.write_text("not json at all")
        self.assertTrue(validate.write_index([entry()]))
        self.assertEqual(json.loads(self.index.read_text())["plugins"], [entry()])


if __name__ == "__main__":
    unittest.main()
