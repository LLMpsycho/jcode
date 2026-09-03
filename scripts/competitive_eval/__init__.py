"""Dependency-free deterministic competitive evaluation harness."""

from .manifest import load_task_manifest, validate_baseline_lock

__all__ = ["load_task_manifest", "validate_baseline_lock"]
