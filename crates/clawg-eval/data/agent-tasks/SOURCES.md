# Sources of the agent tasks

The fixtures and the seeded memories below come from other projects, read at the commit named. Everything a task adds (subject lines, file names in a prompt, checks, ages) is written here, and each entry says which part is whose. A fixture is copied unchanged, because everything inside a fixture directory is what the agent sees and what `search_project` indexes. This file and the licenses stay outside the fixture directories.

`LICENSE-APACHE-2.0` is the license of the Apache-2.0 sources.

## Fixtures

| Fixture | Source | License |
| --- | --- | --- |
| `letta-simple-edit` | `examples/letta-code-simple-edit/init_sandbox/` of https://github.com/letta-ai/letta-evals at `95cef361e33a67d5116598b7be699ea1a0ef04ec` | Apache-2.0 |
| `amb-ts-ignore-gen` | `tasks/ts-ignore-gen/tree/` of https://github.com/GiulioDER/agent-memory-bench at `c1b15726f1153b02dda0490ca1936bb86a809514` | Apache-2.0 |
| `amb-ts-semver-pin` | `tasks/ts-semver-pin/tree/` of the same repository and commit | Apache-2.0 |

## Tasks

**`memorycode-d12-varsuffix-n` (K1).** Memory text from session 0 of `dataset/dialogue_12.json` of https://github.com/for-ai/MemoryCode at `1ab87e119b2f9a498de8075219e1c07f6041b394` (Apache-2.0), without the speaker's name. The prompt ends with that dialogue's evaluation query. Written here: the subject line, `Create palindrome.py with a`, the age and the check. The source's regex requires every variable to end in `_n`; the check accepts one.

**`memorycode-d14-import-secrets` (K2).** Memory text from `dataset/dialogue_14.json` of the same repository and commit, without the speaker's name. The prompt ends with that dialogue's evaluation query. Written here: the subject line, `Create edit_distance.py with a`, the age and the check, which is the source's regex as literal text (it misses `from secrets import ...`).

**`madr-0011-asterisk-list-marker` (K3).** Memory text from lines 9 and 18 of `docs/decisions/0011-use-asterisk-as-list-marker.md` of https://github.com/adr/madr at `ba75bb1b20d42af5746b246ad348c202419ae681` (MIT OR CC0-1.0, taken under CC0-1.0). An ADR has no question and no score: the prompt, the subject line, the age and the checks are written here.

**`amb-ts-ignore-gen-stale` (S).** Prompt from `tasks/ts-ignore-gen/task.json`, memory text from `tasks/ts-ignore-gen/plants/stale_recent_at_top/followup.txt` of agent-memory-bench at the commit above. Only that stale decision is seeded; the benchmark's `superseded` condition also seeds the current one. The check is the whole of `oracles/ts-ignore-gen/expected_gitignore`. Written here: the subject line and the age.

**`amb-ts-semver-pin-conflict` (C).** Prompt from `tasks/ts-semver-pin/task.json`; the older memory is line 6 of `corpus/plants/ts-semver-pin/stale_compatible_release.jsonl` and the newer one line 7 of `corpus/sessions/ts-semver-pin/p01.jsonl`, both of agent-memory-bench at the commit above. Both are seeded live and without a topic key, as two saves the detector did not pair. The checks follow `tasks/ts-semver-pin/checker.py`, with `==` written without spaces. Written here: the subject lines and the ages.

**`letta-simple-edit-task1` (N).** Prompt from the first line of `examples/letta-code-simple-edit/dataset.jsonl` of letta-evals at the commit above, without `sandbox/` and without its second sentence, which only served its grader. Its grader runs the file; the check here reads it.

## Rules and noise

Written here: which seeded memory is a rule and its globs (`rule`), and `user_stated` on the preference of `memorycode-d12-varsuffix-n`, which its dialogue has the person state. `noise.json` is written here entirely: rules on other topics that every arm with memory is seeded with. They stay away from what the checks read (variable names, imports, list markers, `.gitignore`, versions in `requirements.txt`, the signatures of the fixture's functions) and from every tracer, and they are older than 7 days, outside the recent context of clawg's startup.
