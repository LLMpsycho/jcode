from __future__ import annotations

import unittest

from scripts.competitive_eval.compare import compare_results
from scripts.competitive_eval.summarize import summarize_results


class ReportingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.results = [
            {"task_id": "a", "attempt": 1, "agent": "jcode", "status": "pass", "duration_ms": 10},
            {"task_id": "a", "attempt": 1, "agent": "omp", "status": "fail", "duration_ms": 20},
            {"task_id": "b", "attempt": 1, "agent": "jcode", "status": "timeout", "duration_ms": 30},
            {"task_id": "b", "attempt": 1, "agent": "omp", "status": "pass", "duration_ms": 40},
        ]

    def test_summary_reports_statuses_success_rate_and_latency(self) -> None:
        summary = summarize_results(self.results)
        self.assertEqual(summary["total_trials"], 4)
        self.assertEqual(summary["agents"]["jcode"]["statuses"], {"pass": 1, "timeout": 1})
        self.assertEqual(summary["agents"]["jcode"]["success_rate"], 0.5)
        self.assertEqual(summary["agents"]["jcode"]["median_duration_ms"], 20)

    def test_comparison_preserves_paired_task_attempts(self) -> None:
        comparison = compare_results(self.results)
        self.assertEqual(comparison["paired_attempts"], 2)
        self.assertEqual(comparison["outcomes"], {"jcode_win": 1, "omp_win": 1})


if __name__ == "__main__":
    unittest.main()
