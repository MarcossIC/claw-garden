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

A change folder that already holds a `proposal.md`, `spec.md` and `tasks.md` is implemented and reviewed from them as they are; nothing is rewritten into a plan.

**Done first.** Before any code, say what done is as checks a machine can run. One criterion states one thing: a criterion with "and" in it is two criteria. Each names the test or the command that proves it. Say also what must not change: nothing outside the named files or modules, and no existing test weakened or removed.

**Small.** State done and the command that proves it. Write the failing test and run it to see it fail, then the code, then the tests of the files touched. Commit. There is no change folder, and the version takes a patch bump.

**Plan.** A medium or large change has one document, `openspec/changes/<name>/plan.md`, written after reading the code it will touch: every claim about current behavior cites `path:line`. It follows in one file the rules `openspec/config.yaml` gives for proposals, specs, designs and tasks. Sections marked "sólo si" are left out when they do not apply. `state.yaml` records the phase as `plan`, `apply` or `verify`. Implementation starts once the plan is approved, and that approval is the only stop.

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

**One implementer.** One session implements a plan from its first step to its last: the session that wrote it, or a new one that starts from `plan.md`. Implementation is never split across parallel workers. For each step: the failing test, run to see it fail; the code; the tests of the files touched; one commit. Edit `openspec/specs/<area>/spec.md` in the same change, so the diff is the delta and the change folder holds no copy of the spec. Write each deviation from the plan under `Registro` when it happens, not at the end. A large change is implemented in slices, each with its tests, and `Registro` says where the last one stopped, so another session can resume from the file alone.

**Closing checks.** The full suite and both clippy runs go once, on the last commit. A result is reported from the output of the command that produced it.

**Review.** After the closing checks, launch the `change-reviewer` agent with the path of the plan and the range of commits: once for a medium change, twice in parallel for a large one, neither run seeing the other's report. The reviewer reports and never edits; the implementer fixes. With two reviewers, a finding both report is fixed, and a finding only one reports is checked against the code before acting on it. A second round reviews only what changed since the first. When a third round still has findings, stop fixing and put the approach back in front of the person. A critical finding blocks archiving.

**Verification and archive.** Write under `Verificación` what ran and what it returned, each review finding with what was done about it, and what only a local run can check. Then archive as `openspec/README.md` describes.

**Corrections become checks.** When a review or the person corrects the same thing twice, make it a test, a lint or a case in `tests/conventions.rs`, not one more line of instructions.
