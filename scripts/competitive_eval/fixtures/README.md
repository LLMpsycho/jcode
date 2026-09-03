# Deterministic fixtures

`mock/` covers pass, verifier fail, agent timeout, agent crash, unsupported
capability, and capped large-output behavior without a model or competitor.
Every fixture is copied before execution and the source tree remains immutable.
