from __future__ import annotations

import unittest

from scripts.competitive_eval.run_campaign import build_trial_plan


class RandomizedOrderTests(unittest.TestCase):
    def test_order_is_reproducible_for_a_seed(self) -> None:
        first = build_trial_plan(["task-a", "task-b"], ["jcode", "omp"], attempts=5, seed=8128)
        second = build_trial_plan(["task-a", "task-b"], ["jcode", "omp"], attempts=5, seed=8128)
        self.assertEqual(first, second)

    def test_agent_order_is_randomized_per_task_attempt(self) -> None:
        plan = build_trial_plan(["task-a"], ["jcode", "omp"], attempts=20, seed=7)
        orders = [tuple(item["agents"]) for item in plan]
        self.assertIn(("jcode", "omp"), orders)
        self.assertIn(("omp", "jcode"), orders)

    def test_different_seed_changes_order(self) -> None:
        first = build_trial_plan(["task-a"], ["jcode", "omp"], attempts=12, seed=1)
        second = build_trial_plan(["task-a"], ["jcode", "omp"], attempts=12, seed=2)
        self.assertNotEqual(first, second)


if __name__ == "__main__":
    unittest.main()
