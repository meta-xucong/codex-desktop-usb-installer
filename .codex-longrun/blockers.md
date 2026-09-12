# Blockers


## Active

- `longrun_supervisor` runtime is unavailable in this environment; the long-running state machine is being advanced manually with the required state/progress/test records. This does not block code or test execution.
- Real target-machine AppX/Windows cleanup and USB physical write tests cannot be performed from the development workspace; they remain explicit acceptance items after static/package verification.

- Final target acceptance is still device-bound: copy the `r4` bundle to an NTFS removable drive, run the GUI signer there, then test install/update/Skill Hub/offline launch/uninstall. The source and release artifacts have passed the independent-audit and GitHub gates.
