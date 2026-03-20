# TODOS

## Policy

### Add revert/re-evaluation after temporary recovery

**What:** Add a follow-up policy that re-evaluates the original path after a temporary recovery switch and optionally reverts when the original path is healthy again.

**Why:** Prevent the system from staying on an emergency fallback path forever after the original route has recovered.

**Context:** The approved v1 plan already includes failure detection, confirmation probe, temporary select switching, and pending durable suggestions. It does not yet define what should happen after the emergency has passed. This work should build on the coordinator, action log, cooldown logic, and probe budget added in v1 rather than expanding the first implementation.

**Effort:** M
**Priority:** P2
**Depends on:** v1 timeout recovery coordinator, action log, and probe budget
