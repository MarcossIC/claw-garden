# protected-verdicts — Verificación

## Linux (2026-10-20)

Sobre `bc9b9a6`, en `target`, con el compilador de `rust-toolchain.toml`:

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` sale con 0.
- Cada commit de código de las [tareas](tasks.md) (`4f4aee1`, `000e331`, `f3fb246`) pasó la suite y clippy antes de commitearse.
- `the_agent_contract_matches_its_file` cambió a propósito y se bendijo: las instrucciones (la frase de ACTIVE CONFLICTS), las descripciones de `resolve_conflict` y `update_memory`, y `user_answered` en sus dos schemas. Nada más del contrato se movió.

## El binario real

Con `target/debug/clawg` de un build de tests, `CLAWG_TEST_HOME` en un directorio temporal, `CLAWG_TEST_EMBEDDER=fake` y `CLAWG_TEST_DEFAULT_PORT=off`, un daemon `dev` en `--port 0`: un save `user_preference` y después

```
PATCH /v1/memories/memory/1 {"memory_type":"semantic_fact"}
409 {"error":"memory 1 is a user_preference, so it loses that type only on the user's word: ask the user, then call again with `user_answered`"}
PATCH /v1/memories/memory/1 {"memory_type":"semantic_fact","user_answered":true}
200 {"updated":1,"id":1,"kind":"memory","importance":5.0,"memory_type":"semantic_fact","tier":3}
```

y `clawg stop` lo para.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| R1, R2 | Los tests de `dispute.rs` no compilaban contra la firma de `judge_pair` sin `user_answered`; la secuencia `conflicts` → `supersedes` que fijaba el test anterior se reemplazaba. |
| R3 | `demoting_a_protected_row_needs_the_answer` no compilaba sin `UpdateError` ni `MemoryPatch::answered`; el `PATCH` bajaba el tipo con `200`. |
| R4 | Los tests del puente no compilaban sin el campo en `ResolveConflictArgs` y `UpdateMemoryArgs`. |

`rag-eval` y `conflict-eval` no aplican: no cambian ranking, vectores, prompt ni detector.

## Pendiente

- El CI del pull request en Linux y Windows.
