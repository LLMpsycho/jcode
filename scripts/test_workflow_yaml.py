#!/usr/bin/env python3
"""Reject duplicate YAML keys before GitHub discards an entire workflow.

Requires PyYAML. This is structural YAML validation, not an Actions schema or
expression validator; nested duplicate mappings are rejected as well as env.
"""
from pathlib import Path
import unittest

import yaml


class UniqueKeyLoader(yaml.BaseLoader):
    """Keep Actions' `on` as text and reject keys that YAML otherwise overwrites."""


def unique_mapping(loader, node):
    mapping = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=True)
        if key in mapping:
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping", node.start_mark,
                f"duplicate key: {key!r}", key_node.start_mark,
            )
        mapping[key] = loader.construct_object(value_node, deep=True)
    return mapping


UniqueKeyLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, unique_mapping
)


def load_workflow(text):
    return yaml.load(text, Loader=UniqueKeyLoader)


class WorkflowYamlTests(unittest.TestCase):
    def test_duplicate_top_level_env_is_rejected(self):
        with self.assertRaisesRegex(yaml.YAMLError, "duplicate key: 'env'"):
            load_workflow("env:\n  JCODE_CI: '1'\nenv:\n  CARGO_TERM_COLOR: always\n")

    def test_duplicate_nested_keys_are_rejected(self):
        with self.assertRaisesRegex(yaml.YAMLError, "duplicate key: 'run'"):
            load_workflow("steps:\n  - run: first\n    run: overwritten\n")

    def test_on_and_distinct_env_scopes_are_preserved(self):
        workflow = load_workflow("on: [push]\nenv: {X: one}\njobs: {test: {env: {X: two}}}\n")
        self.assertEqual(workflow["on"], ["push"])
        self.assertEqual(workflow["jobs"]["test"]["env"]["X"], "two")

    def test_every_repository_workflow_has_unique_keys(self):
        root = Path(__file__).resolve().parents[1]
        workflows = sorted((root / ".github/workflows").glob("*.y*ml"))
        self.assertTrue(workflows, "no workflow files discovered")
        for path in workflows:
            with self.subTest(workflow=path.name):
                workflow = load_workflow(path.read_text(encoding="utf-8"))
                self.assertIsInstance(workflow, dict)
                self.assertIn("on", workflow)
                self.assertIn("jobs", workflow)

    def test_repaired_workflows_keep_ci_and_other_environment_values(self):
        root = Path(__file__).resolve().parents[1] / ".github/workflows"
        for name in ("ci.yml", "windows-smoke.yml", "ios-testflight.yml"):
            with self.subTest(workflow=name):
                env = load_workflow((root / name).read_text(encoding="utf-8"))["env"]
                self.assertEqual(env["JCODE_CI"], "1")
                self.assertGreater(len(env), 1)


if __name__ == "__main__":
    unittest.main()
