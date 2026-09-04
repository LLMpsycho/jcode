"""Agent adapters for the competitive evaluation harness."""

from .base import AgentAdapter
from .jcode import JcodeAdapter
from .mock import MockAdapter
from .omp import OmpAdapter

__all__ = ["AgentAdapter", "JcodeAdapter", "MockAdapter", "OmpAdapter"]
