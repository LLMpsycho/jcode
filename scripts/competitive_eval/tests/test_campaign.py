from __future__ import annotations

import json
import tempfile
import unittest
from collections import Counter
from pathlib import Path

from scripts.competitive_eval.run_campaign import REPO_ROOT, run_campaign


FIXTURES = REPO_ROOT / "scripts" / "competitive_eval" / "fixtures" / "mock"


class CampaignTests(unittest.TestCase):
    def test_dry_run_needs_no_competitor_binary(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            campaign_dir, results = run_campaign(
                [FIXTURES / "pass" / "task.json"],
                agents=["jcode", "omp"],
                attempts=1,
                seed=19,
                results_root=Path(tmp),
                campaign_id="dry-run",
                dry_run=True,
            )
            campaign = json.loads((campaign_dir / "campaign.json").read_text(encoding="utf-8"))
            plan = json.loads((campaign_dir / "plan.json").read_text(encoding="utf-8"))
        self.assertEqual(results, [])
        self.assertTrue(campaign["dry_run"])
        self.assertEqual(len(campaign["task_manifest_sha256"]), 64)
        self.assertEqual(len(campaign["verifier_sha256"]), 64)
        self.assertCountEqual(plan[0]["agents"], ["jcode", "omp"])

    def test_one_failure_does_not_abort_remaining_mock_trials(self) -> None:
        manifests = [FIXTURES / name / "task.json" for name in ("pass", "fail", "crash", "unsupported")]
        with tempfile.TemporaryDirectory() as tmp:
            campaign_dir, results = run_campaign(
                manifests,
                agents=["mock"],
                attempts=1,
                seed=3,
                results_root=Path(tmp),
                campaign_id="continue-after-failure",
                output_limit_bytes=2048,
            )
            stored = json.loads((campaign_dir / "results.json").read_text(encoding="utf-8"))
        self.assertEqual(len(results), 4)
        self.assertEqual(len(stored), 4)
        self.assertEqual(
            Counter(result["status"] for result in results),
            Counter({"pass": 1, "fail": 1, "crash": 1, "unsupported": 1}),
        )


if __name__ == "__main__":
    unittest.main()
