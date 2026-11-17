# Releasing Develop

## Hard rule: every release updates all the documentation

No release goes out until every document reflects the app as it is in that release. This is
not optional, and it applies to people and to AI assistants alike. The release workflow checks
it and refuses to build a release that breaks it (see [The automatic check](#the-automatic-check)).

Before tagging a release, go through **every** document below and bring it up to date with
everything new, changed or removed since the previous release:

| Document | What to update |
|---|---|
| `RELEASE_NOTES.md` | Starts with `## Develop <version>`. What is new, what changed, fixes, known limitations, and the install steps with this version's file names. It is the GitHub release description and appears in the app's update dialog, so write it for photographers, not developers. |
| `docs/USER-GUIDE.md` | Every feature, panel, option, shortcut and file location as it now is. New features get their own section; changed ones are rewritten; removed ones are taken out. |
| `README.md` | The overview, "What it does well" / "What it does not have", the Features list, supported cameras and formats, install and build instructions. |
| `docs/MCP.md` and `docs/develop-skill/SKILL.md` | Whenever the MCP server or what it can do changed: tools, settings, rules, examples, setup. Run the examples against the real server. |
| `models/README.md`, `lensfun/README.md`, `lightroom/` | When the AI models, lens database or reference plugin changed. |

Every statement must be checked against the code or the running app, not written from memory.

## Steps

1. **Code done and tested:** `cargo test --release` passes; the change was tried in the app.
2. **Documentation:** update every document as above.
3. **Version:** set `version` in `Cargo.toml` (and let `Cargo.lock` follow with a build). Ask the
   owner which version number to use if it is not obvious.
4. **Commit** the code, documentation and version together, with a message describing the release.
5. **Tag and push:** `git tag v<version>` then `git push` and `git push origin v<version>`.
6. **Watch the build** (GitHub Actions, "Release"): the draft is created, the Windows installer
   and the macOS disk image are built and attached, then the release is published.
7. **Verify** the published release: both installers attached, notes shown, and an older copy of
   Develop offers the update.

## The automatic check

The release workflow's first job fails, and no release is made, unless:

- `RELEASE_NOTES.md` starts with `## Develop <version>` for the tag being released;
- `Cargo.toml` has that version;
- `RELEASE_NOTES.md`, `README.md` and `docs/USER-GUIDE.md` all changed since the previous release
  tag; and
- if the MCP server (`src/mcp.rs`, `src/measure.rs`) changed since the previous release,
  `docs/MCP.md` and `docs/develop-skill/SKILL.md` changed too.

The check can only see that files changed, not that they are complete: that part is the rule
above.
