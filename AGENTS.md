# Sparkplane contributor contract

Sparkplane owns the DGX Spark appliance, client, engines, models, qualification,
signed releases and documentation. It must build without sy or Sparky. The sy
integration is an external process bridge, not a source or library dependency.

- Read README.md, SECURITY.md and the relevant operator guide before changes.
- Test behavior first; retain real IPC/HTTPS coverage and the client-only gate.
- Run `make lint`, `make test`, `make test-client` and `make audit`.
- Keep stdout machine-readable for `--json`; logs belong on stderr.
- Never weaken signed inventory, peer authorization, resource admission,
  confinement or exact-container cleanup to make a test pass.
- New root operations must be finite typed actions, never caller-selected argv.
- Do not rewrite content-addressed patches or historical signed evidence.
- Preserve the optimized engine's context and settings during namespace work.
- Live migration requires a verified trust transition and explicit cutover;
  never restart Docker or reboot as a deployment shortcut.
- Keep credentials and runtime artifacts out of commits and logs.
- Do not use subagents unless the user explicitly requests them.
