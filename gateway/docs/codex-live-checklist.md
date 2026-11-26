# Checking the gateway against a real Codex

The gateway's tests use a mock app-server. They check that the gateway sends what the protocol
defines (`tests/protocol_contract.rs`), but not that real Codex behaves as the code assumes. This
checklist covers the assumptions that have not been checked against a real Codex yet. Run it on a
machine that has a working `codex` app-server and a model key, with the gateway pointed at it.

Each item says what to do, what the gateway assumes, and what to change if Codex disagrees.

## Setup

1. Build the gateway and start it against a Postgres database, with a local environment that can
   launch Codex (see `docs/deploy.md`).
2. Sign in, create a workspace and a local environment, and check that `GET /environments/{id}/models`
   lists a model.
3. For code items, create a code chat on a small git repository with a remote, so there is a
   checkout and a branch to compare with.

## 1. Workspace diff (`gitDiffToRemote`)

- Do: `GET /threads/{id}/workspace-diff` on a code chat after editing a file.
- Assumed: `{ sha, diff }`, where `diff` includes uncommitted files and files changed by shell commands.
- Check: the diff lists the edited file; `sha` is the remote branch's commit.
- If not: an absent `sha` or `diff` makes the gateway return 404 ("no changes codex can compare").

## 2. Undoing a turn (`thread/revert`)

- Do: make a turn that edits a file, then `POST /turns/{id}/revert` on that turn.
- Assumed: codex rewinds its own history, and the gateway marks the turn and later turns `reverted_at`.
- Check, and this is the important one: is the edited file restored to its state before the turn?
  Look at the checkout on disk.
- If the file is not restored: the "Revert to before" button must not promise it. Change its
  confirmation text and the revert handler, or revert the files in the gateway with `git`.
- Also check: after reverting, the next turn starts from the rewound state (ask the model what it
  remembers from the reverted turn).

## 3. Interactive commands (`command/exec` with `streamStdin`, `tty`; `command/exec/write`; `command/exec/resize`)

- Do: run `cat` with "Takes input" on, type a line, then "Close input". Run `sh` (or any prompt) with
  "In a terminal" on, and set the size.
- Assumed: `outputDelta` names only `processId` (the hub routes by it); a closed input ends the
  command; a terminal's output arrives on `stdout`.
- Check: the typed text appears in the output; the command exits after the input closes; resizing
  a terminal does not error.
- If not: a missing `processId` on `outputDelta` means output is dropped (`hub.rs`,
  `record_process_output`); a command that does not end on close means `stopped` must be used instead.

## 4. Suggestions (`thread/prediction/request`, `thread/prediction/updated`)

- Do: `POST /turns/{id}/prediction` on a finished turn.
- Assumed: the result arrives as `thread/prediction/updated` with `result.type` `completed` and
  `result.text`, or `failed`.
- Check: a suggestion appears above the message box. Note whether the request costs a model call and
  how long it takes.
- If not: the field names in `ThreadPredictionResult` are read in `useCodex.ts`.

## 5. Goals (`thread/goal/get|set|clear`, `thread/goal/updated`)

- Do: set a goal with a token budget, pause and resume it, then clear it. Send a message that makes
  codex work toward it.
- Assumed: status values are `active`, `paused`, `blocked`, `usageLimited`, `budgetLimited`,
  `complete`; `tokenBudget: null` removes a budget; updates arrive as `thread/goal/updated`.
- Check: the goal bar shows the progress (tokens and time used) and updates while codex works.
- Important: an active goal makes codex keep working without a new message. Confirm that is the
  behaviour you want, and that `budgetLimited` stops it.

## 6. Background terminals (`thread/backgroundTerminals/list|terminate|clean`)

- Do: ask codex to start a long-running process in a code chat (for example `sleep 600`), then list
  the terminals, stop one, and clean up.
- Assumed: `data` with `processId`, `command`, `cwd`, `osPid`, `cpuPercent`, `rssKb`; `terminated`
  is `true` for a stopped process.
- Check: the process appears, and is gone after stopping (`ps`).
- These methods are experimental and not in the bundled schema. Their field names come from the Rust
  source, not from a schema, so confirm them here.

## 7. Files (`fs/writeFile`, `fs/createDirectory`, `fs/copy`, `fs/remove`, `fs/watch`, `fs/changed`)

- Do: create, rename and delete a file from the file tree. Then have codex edit a file in a turn while
  the tree is open.
- Assumed: `fs/watch` reports changes as `fs/changed` with `watchId` and `changedPaths`.
- Check: the tree refreshes after codex edits files. Confirm that a file write is visible on disk.

## 8. Environment notices (`account/rateLimits/updated`, `skills/changed`, `configWarning`, `warning`, `deprecationNotice`, `model/rerouted`)

- Do: change a skill in the workspace and check the skills list; provoke a configuration warning
  (for example an unknown key in the config file).
- Assumed: notices with no `threadId` concern the environment; the gateway forwards them to browsers
  watching a chat in that environment.
- Check: a warning appears as a banner; the skills list refreshes.

## 9. Limits, memory and MCP status (`account/rateLimits/read`, `memory/status`, `memory/reset`, `mcpServerStatus/list`)

- Do: open the usage dialog, the memory settings (as an admin, then as a member), and the plugins
  page's "Check now".
- Assumed: `rateLimits` with `primary` and `secondary`; `v2ConsolidatedThreads` and `v2Ready`;
  `runtimeStatus` values such as `connected` and `authenticationRequired`.
- Check: the values match what codex shows (`codex` CLI status, or the account page).
- Check: a member gets 403 on `memory/reset`, and the reset clears the count.

## 10. Revert and fork together

- Do: revert a turn, then fork the chat from a turn before it.
- Check: the fork does not contain the reverted turns (`fork_thread` skips them).

## 11. Config, features and member runtime calls (`config/value/write`, `config/batchWrite`, `config/mcpServer/reload`, `experimentalFeature/*`, `skills/config/write`, `server/diagnostics`, `thread/settings/update`, `hooks/list`, `mcpServer/tool/call`, `mcpServer/resource/read`)

- Do: as an admin, read the config requirements, set a harmless value (for example `features.goals`), reload the MCP
  servers, toggle an experimental feature, switch a skill off and on, and open the diagnostics. As a member, change the
  chat's model and effort, list the hooks, call an MCP tool, and read an MCP resource.
- Assumed: `config/value/write` and `config/batchWrite` answer with `ConfigWriteResponse` (the batch response type is
  inferred, not in the schema). `config/mcpServer/reload` answers with no result type.
- Not in the bundled schema, so confirm them here: `thread/settings/update` takes `threadId`, `model` and `effort`
  (the schema has `effort` only on the `ThreadSettings` notification); `server/diagnostics` returns `process` and
  `gauges`; `hooks/list` accepts `cwds: []`.
- Check: an edit appears in the config file, and a secret-looking key is refused by the gateway, not by codex. A member
  gets 403 on every admin route. The audit log shows key paths and tool names, never values or arguments.
- If not: fix the parameter names in `routes/thread_runtime.rs` and `routes/codex_config.rs`, and the matching mock arms.

## After the checks

- Record the result of each item in a short note: "as assumed", or what Codex actually sent.
- Where Codex differs, fix the gateway or the web app, then add the real message as a fixture in the
  mock so the difference is tested from then on.
- Update this checklist: remove items that are now verified.
