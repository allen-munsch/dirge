# backpressured

A dirge port of the **backpressure loop** ([lucasfcosta/backpressured](https://github.com/lucasfcosta/backpressured),
[the essay](https://www.lucasfcosta.com/blog/backpressure-is-all-you-need)):
drive a goal to completion autonomously while making the **machine say "no"
first** — lint, tests, typecheck, and an independent reviewer gate *every
iteration*, instead of a human catching mistakes at the end.

> Any system that relies on a human to catch the machine's mistakes is
> limited by the human, not the machine.

## What it does

While engaged, the plugin appends a four-phase loop discipline to the
system prompt:

1. **Plan** → an independent `task` reviewer approves the approach before any code.
2. **Implement in a loop** → after *every* patch: lint, targeted tests (with new behavior covered), typecheck/build, and a `task` reviewer on the diff. Don't write the next patch until green.
3. **Before "done"** → full suite, then *run it for real* (curl / browser), holistic reviewer over the whole changeset.
4. **Ship** → open the PR and watch it to merge (or merge from a local branch).

The independent reviewer is always a **`task` subagent that did not write the
code** — your own re-read shares the blind spots you wrote with.

## Install

dirge auto-discovers plugins from two directories at startup. Copy this
directory into one of them (it loads as a multi-file plugin — its `*.janet`
files share one Janet env):

```
~/.config/dirge/plugins/backpressured/     # global — every project
<project>/.dirge/plugins/backpressured/    # per-project — wins on collision
```

For example, from a checkout of the dirge repo:

```bash
cp -r plugins/backpressured ~/.config/dirge/plugins/
```

Requires building with the `plugin` feature (on by default). Confirm it
loaded with `dirge --verbose` (prints `loading plugin: …/backpressured`) or
by running `/backpressured-status`. The plugin stays dormant until you
engage it (see Usage).

## Usage

Engage it by mentioning **backpressure** in your prompt:

```
backpressured: add a /health endpoint with a test and wire it into routing
```

…or with the command:

```
/backpressured add a /health endpoint with a test
/backpressured-status     # show state + detected project checks
/backpressured-stop       # disengage
```

The keyword form is the reliable trigger — the prompt flows to the model
normally and the loop discipline is injected for that run and subsequent
ones until you stop it.

## Project checks

The plugin auto-discovers your check commands from the project manifest
(`Cargo.toml`, `package.json`, `deps.edn`/`project.clj`, `pyproject.toml`,
`go.mod`, `Makefile`) and names them in the discipline.

To customize, drop a **`BACKPRESSURE.md`** at your project root with
plain-English, project-specific instructions (exact lint/test commands, how
to run the app, what to skip, shipping style). It's **authoritative** — when
present it's handed to the loop verbatim and wins over the auto-detected
defaults.

## Files

| File | Purpose |
|------|---------|
| `00-state.janet` | mode flag + project check discovery |
| `01-hooks.janet` | keyword trigger (`on-prompt`) + discipline injection (`before-agent-start`) |
| `02-commands.janet` | `/backpressured`, `/backpressured-stop`, `/backpressured-status` |

## Notes & limitations

- **Checks run through the agent's normal `bash` tool**, so long test
  suites stream and the permission engine still applies. (A future
  *enforced* gate — the plugin running checks itself in `prepare-next-run`
  and refusing to finish until green — is possible via dirge's
  `harness/request-prompt`, but is deferred: running a full suite
  synchronously in a hook would block the plugin worker.)
- The reviewer is dispatched by the agent via the `task` tool (prompt-driven),
  same as the original — plugins can't spawn subagents directly.
- For UI manual-testing, add a Playwright MCP server to your dirge config.
