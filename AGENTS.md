# Project scope and implementation rules

- Stay within scope: Implement only my explicit request; do not expand the task or prepare for unrequested future requirements.
- Follow the project: Inspect and follow the project’s existing code, conventions, and actual call paths; reuse existing mechanisms and make the smallest sufficient change.
- No SHA checks: Do not add or run SHA/hash/checksum generation or verification unless I explicitly request it.
- Fallbacks require a proven consumer: Add a fallback only when concrete project evidence conclusively confirms that a real, existing downstream consumer requires that exact fallback behavior; identify the consumer, its call path, and the dependency—if any of these is unverified, do not add the fallback.
- Edge-case handling requires P0 impact: Add edge-case handling only for a verified P0 issue with major impact, supported by a concrete, reachable failure scenario and the project’s established severity criteria; do not invent severity criteria, inflate severity, or cover hypothetical or non-P0 cases.
- No speculative design or defense: Do not introduce speculative abstractions, defensive layers, future-proofing, or duplicate safeguards; every addition must be necessary for the requested behavior.
- Keep validation proportional: Run the project’s required checks and the minimum additional validation needed to verify the requested behavior; do not add redundant checks or broaden testing without a concrete, task-related reason.
- Do not authorize your own scope expansion: “Best practice,” “extra safety,” and your own assumptions about potential risks do not constitute authorization; leave optional work unimplemented and continue the requested task.
