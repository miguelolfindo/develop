# Instructions for AI assistants working on Develop

## Hard rule: documentation is updated for every release

Before any release is tagged, **all** documentation must be brought up to date with every change
since the previous release: `RELEASE_NOTES.md`, `docs/USER-GUIDE.md`, `README.md`, and, when the
MCP server changed, `docs/MCP.md` and `docs/develop-skill/SKILL.md`. Check every statement against
the code or the running app. Follow [RELEASING.md](RELEASING.md) step by step. The release workflow
enforces this and fails without it; do not work around the check.

## Other rules

- Commit and push only when the owner asks. Ask before publishing a release, and ask which version
  number to use if it is not obvious.
- Release notes and user documentation are for photographers: plain words, no jargon.
- Test with scratch copies of photos and a scratch settings folder (`APPDATA` pointing elsewhere),
  never the owner's real photo library or settings.
