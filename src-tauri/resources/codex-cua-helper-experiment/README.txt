CUA MTA HELPER EXPERIMENT — 2026-11-01

This is an isolated experimental USB bundle. The included v3 computer-use helper is modified and is not an OpenAI-signed component. It is selected only by this bundle's dedicated SignedRequired experiment manager at resources\codex-cua-helper-experiment\codex-computer-use-mta-wrapper-candidate-v3.exe.

This static package intentionally contains no usb-license.json. Run it only after the exact bundle has been copied to a dedicated test USB volume and a matching license has been issued for that volume and bundle anchor.

For the test: save and close any current Codex normally; start this bundle with CodexDesktopSetup.exe. Open a blank Notepad window normally. In the new trusted Codex CUA session, use the supported window enumeration and get_window flow to bind that window; this initializes the service but does not capture or interact with it. Then confirm the helper process image path points to this bundle's candidate file before requesting one screenshot. If the normal OpenAI helper appears or the candidate does not start, stop. Only after the first screenshot succeeds, one harmless visible Notepad action and one further screenshot are in scope.

To roll back, close the experiment manager and Codex normally, then launch the untouched previous USB bundle from its own directory. Do not merge this directory into or overwrite the previous bundle.