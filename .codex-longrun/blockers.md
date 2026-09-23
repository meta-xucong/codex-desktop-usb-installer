# Blockers


## Active

- `longrun_supervisor` runtime is unavailable in this environment; the long-running state machine is being advanced manually with the required state/progress/test records. This does not block code or test execution.
- Real target-machine AppX/Windows cleanup and USB physical write tests cannot be performed from the development workspace; they remain explicit acceptance items after static/package verification.

- Final target acceptance is still device-bound: copy `release/usb-oneclick-g39.5-20260922-r8-final` to an NTFS removable drive, run `release/usb-license-signer-gui-v0.5.24-g39.5-r7-20260922-final-embedded` there, then test sign/register/install/model-refresh/update/Skill Hub/offline launch/uninstall. The source, complete package, signer package and profile pair have passed local verification.
- This workstation currently has an unrelated external `D:\AI\cc-switch\app\cc-switch.exe` holding CCSwitch's single-instance listener on port 15721; an isolated final-package launch must be repeated after that external instance is closed.
- The r16 signer source and tests pass, but the production signer EXE was intentionally not replaced because this environment does not expose the original compile-time administrator Key. A no-key replacement is unsafe; rebuild it only in the secured signing environment that owns that credential.
