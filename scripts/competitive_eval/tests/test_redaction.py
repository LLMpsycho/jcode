from __future__ import annotations

import unittest

from scripts.competitive_eval.redact import REDACTED, redact_mapping, redact_text


class RedactionTests(unittest.TestCase):
    def test_sensitive_environment_keys_and_nested_values_are_redacted(self) -> None:
        data = {
            "PATH": "/usr/bin",
            "OPENAI_API_KEY": "sk-test-super-secret",
            "nested": {"password": "hunter2", "safe": "visible"},
        }
        redacted = redact_mapping(data)
        self.assertEqual(redacted["PATH"], "/usr/bin")
        self.assertEqual(redacted["OPENAI_API_KEY"], REDACTED)
        self.assertEqual(redacted["nested"]["password"], REDACTED)
        self.assertEqual(redacted["nested"]["safe"], "visible")

    def test_text_redacts_known_values_bearer_tokens_urls_and_key_assignments(self) -> None:
        text = (
            "token=plain-secret Authorization: Bearer abc.def.ghi "
            "https://user:pass@example.test/path?api_key=query-secret known-value"
        )
        redacted = redact_text(text, secrets=["known-value", "plain-secret"])
        for secret in ("plain-secret", "abc.def.ghi", "user:pass", "query-secret", "known-value"):
            self.assertNotIn(secret, redacted)
        self.assertIn(REDACTED, redacted)

    def test_short_or_empty_known_values_are_not_globally_replaced(self) -> None:
        self.assertEqual(redact_text("a cat", secrets=["", "a"]), "a cat")


if __name__ == "__main__":
    unittest.main()
