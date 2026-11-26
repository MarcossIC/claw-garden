---
name: change-workflow
description: "Proportion. Use when starting, planning, implementing, reviewing or archiving a change to this repository: sizes the work as small, medium or large and gives each size its plan, its implementation loop and its review."
---

# Change Workflow

**Proportion.** The size of a change decides how much structure it carries. Size it before writing anything, and when two sizes fit, take the larger.

| Size | It is this size when | Document | Review |
|---|---|---|---|
| Small | It changes nothing `openspec/specs/` describes, touches no stored data, ranking or pinned MCP contract, and fits in one or two commits | none | the closing checks |
| Large | It touches stored data (schema, stored identifiers, statuses, the embedding contract), ranking or retrieval, or the pinned MCP contract; or more than one approach is reasonable; or where the change lives is not yet known | `plan.md` with its conditional sections | two reviewers |
| Medium | Everything else | `plan.md` | one reviewer |

**Queue.** A change waits as an `exploration.md` and a `state.yaml` in its folder, in the order `openspec/ROADMAP.md` gives. When it is taken, its plan is written from the exploration, in the same folder. A change folder that already holds a `proposal.md`, `spec.md` and `tasks.md` is implemented and reviewed from them as they are; nothing is rewritten into a plan.

**Older folders.** In such a folder the three documents stand in for `plan.md` everywhere below. A review is given the three paths in place of the plan's path, and its criteria are the requirements of `spec.md`. The plan review covers only what `tasks.md` has not yet checked off; it approves the same way, `CLEAN` or `WARNINGS`. Approval, decisions taken without asking and deviations go under a `## Registro` heading at the end of `tasks.md`. What goes under `Verificación` goes in a `verify.md` beside them.

**Done first.** Before any code, say what done is as checks a machine can run. One criterion states one thing: a criterion with "and" in it is two criteria. Each names the test or the command that proves it. Say also what must not change: nothing outside the named files or modules, and no existing test weakened or removed.

**Small.** State done and the command that proves it. Write the failing test and run it to see it fail, then the code, then the tests of the files touched. Commit. There is no change folder, and the version takes a patch bump.

**Plan.** A medium or large change has one document, `openspec/changes/<name>/plan.md`, written after reading the code it will touch: every claim about current behavior cites `path:line`. It follows the `general` and `plan` rules of `openspec/config.yaml`. Sections marked "sólo si" are left out when they do not apply. `state.yaml` records the phase as `plan`, `apply` or `verify`. Implementation starts once the plan is approved. With the person in the conversation, they approve it, and that approval is the only stop. In autonomous work the plan review approves it, and the session goes on without waiting.

**Autonomous work.** A session works autonomously when it runs in the background (`claude --bg`, a scheduled task, a cloud session), when another session launched it to orchestrate, or when the person said to carry on alone. An autonomous session never waits for an answer: it finishes the work, or leaves it closed and explained.

- The plan review approves the plan. Once `plan.md` is written, launch the reviewers as under **Review**, in plan mode: the message says "plan mode" and gives the path of the plan (or the three documents of an older folder) and the original request, quoted. A medium change has `clawg review` alone; a large one has `clawg review` and the `change-reviewer` agent in parallel, neither seeing the other's report. Findings are handled as under **Review**. When a round returns `CLEAN` or `WARNINGS`, the plan is approved: note it under `Registro` and implement it.
- What would need a question is decided by the narrowest reading of the request and noted under `Registro` with its reason. Nothing the request does not name is added to settle a doubt.
- When a third round, of the plan or of the change, still has findings, stop fixing and do not wait: leave the work committed on its branch, unmerged, with the open findings under `Verificación`, and end with them in the final answer.
- What truly needs the person (deleting or migrating stored data that cannot be rebuilt, a release, touching the real data profiles, permissions or credentials) is not done: implement the rest and end by saying what was left out and why.
- Whoever orchestrates writes no stops into the request. The request of an autonomous session says it works autonomously by this skill, and never says "stop to show me the plan": a stop written into the request is honoured, and another session cannot lift it.
- The final answer says what was approved and in how many rounds, the commits and the branch, what was decided without asking, and what is left to check by hand.

```markdown
# <nombre> — Plan

Corre en: nube | local · Necesita a la persona: no | para decidir … | para correr …

## Problema
Qué pasa hoy, con la evidencia: `ruta:línea`, un test que falla o una medición.

## Objetivo y fuera de alcance
Qué cambia, a qué objetivo del producto sirve y qué no se toca aunque esté cerca.

## Hecho
- H1. Dado … cuando … entonces … → `archivo::test`
- No cambia nada fuera de …
- No se afloja ni se borra ningún test existente.

## Medición
Sólo si toca ranking o rendimiento: métrica, línea base y umbral, fijados antes de medir.

## Decisión
Sólo si hay más de un enfoque: el elegido, las alternativas y por qué.

## Datos y vuelta atrás
Sólo si toca datos guardados: la migración, el test que siembra la versión anterior y cómo se vuelve.

## Pasos
1. <test que falla> → <cambio>. Un paso, un commit.

## Registro
Lo que se hizo, lo que se desvió y por qué, y dónde quedó.

## Verificación
Qué se corrió y qué devolvió, los hallazgos de la revisión y lo pendiente en local.
```

**Exploration burst.** Only for a large change, and only before its plan: two to four read-only subagents in parallel on a cheaper model, each with one narrow question, each returning short findings with `path:line`. The session that plans reads the findings and writes the plan itself. Nowhere else does a change add agents.

**One implementer.** One session implements a plan from its first step to its last: the session that wrote it, or a new one that starts from `plan.md`. It follows the `apply` rules of `openspec/config.yaml`. Implementation is never split across parallel workers. For each step: the failing test, run to see it fail; the code; the tests of the files touched; one commit. Edit `openspec/specs/<area>/spec.md` in the same change, so the diff is the delta and the change folder holds no copy of the spec. Write each deviation from the plan under `Registro` when it happens, not at the end. A large change is implemented in slices, each with its tests, and `Registro` says where the last one stopped, so another session can resume from the file alone.

**Closing checks.** The full suite and both clippy runs go once, on the last commit. A result is reported from the output of the command that produced it.

**Review.** After the closing checks, the session that implemented, the orchestrator, launches the reviewers. The first reviewer is always `clawg review`, a model outside Claude; the `change-reviewer` agent is only ever the second:

- A medium change is reviewed by `clawg review` alone.
- A large change is reviewed by `clawg review` and the `change-reviewer` agent in parallel, neither seeing the other's report.
- Without the Cursor CLI (a cloud session, for example), `change-reviewer` takes the place of `clawg review`.

`clawg review` runs the Cursor CLI as a reviewer that reads the code and the project's memory but writes nothing, and records its session. It follows the same instructions as `change-reviewer`, plan mode included. Run it in the background, from the repository:

- `--instructions .claude/agents/change-reviewer.md`;
- on stdin, a message with the path of the plan (or the three documents of an older folder), the range of commits, and what the closing checks ran and returned; for a plan review, "plan mode", the path of the plan and the original request, quoted.

The model is the one `clawg preferences set review-model` chose, for the profile or the project, or `gpt-5.3-codex-high`. Its report ends with `CLEAN`, `WARNINGS` or `BLOCKED`, and its last line names the session. After each review, check that `git status` shows nothing the review left.

The reviewers report and never edit, and the orchestrator decides what to do with each finding: it fixes it, or checks it against the code before acting on it. With two reviewers, a finding both report is fixed, and a finding only one reports is checked against the code before acting on it. A second round reviews only what changed since the first: `clawg review --resume <session>` goes back to the same reviewer, with the message saying what changed. When a third round still has findings, stop fixing and put the approach back in front of the person, or, in autonomous work, close as **Autonomous work** says. A critical finding blocks archiving.

**Verification and archive.** Write under `Verificación` what ran and what it returned, each review finding with what was done about it, and what only a local run can check, following the `verify` rules of `openspec/config.yaml`. Then archive as `openspec/README.md` and the `archive` rules describe.

**Corrections become checks.** When a review or the person corrects the same thing twice, make it a test, a lint or a case in `tests/conventions.rs`, not one more line of instructions.
