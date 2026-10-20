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

## CI (2026-10-20)

El CI del pull request #66 (run 39408972625) pasó sobre `b4ea6af`, la punta de la rama, en Linux (`test (ubuntu-latest)`) y en Windows (`test (windows-latest)`): clippy y la suite en los dos. Su merge en `develop` (`85cab5f`) tiene el mismo árbol que `b4ea6af`.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| R1, R2 | Los tests de `dispute.rs` no compilaban contra la firma de `judge_pair` sin `user_answered`; la secuencia `conflicts` → `supersedes` que fijaba el test anterior se reemplazaba. |
| R3 | `demoting_a_protected_row_needs_the_answer` no compilaba sin `UpdateError` ni `MemoryPatch::answered`; el `PATCH` bajaba el tipo con `200`. |
| R4 | Los tests del puente no compilaban sin el campo en `ResolveConflictArgs` y `UpdateMemoryArgs`. |

`rag-eval` y `conflict-eval` no aplican: no cambian ranking, vectores, prompt ni detector.

## Al archivar

Los requisitos pasan a [`specs/disputes`](../../../specs/disputes/spec.md) (R1 y R2, en R3, con «Declaración» en las definiciones, el paso de `conflicts`, R6 y «Datos de la versión anterior»), [`specs/update`](../../../specs/update/spec.md) (R3, en R1, R4 y R5, con «Fila protegida» y «Declaración» en las definiciones) y [`specs/mcp`](../../../specs/mcp/spec.md) (R4, en «Los conflictos en las herramientas» y la apertura). En los tres, cada cita a `dispute.rs`, `admin.rs`, `store_insert.rs`, `api.rs`, `client.rs` y `server.rs`, que cambiaron desde `e57a563` con este cambio y con windows-socket, se verificó contra `4f7170f`: se corrigieron 102. Dos ya apuntaban mal en `e57a563` y se resolvieron por el símbolo: `Health` en `api.rs:494`, un comentario, es `HealthBody` y su campo `embedder_resident` (`api.rs:499`); `RetrievalState::of` en `api.rs:1351`, un `});`, está en `api.rs:673`. Diferencias entre los docs del cambio y el código:

1. **`is_protected` no es la única lista.** El diseño (D3) la llama la única lista de tipos protegidos, pero `needs_user` tiene la suya (`crates/clawg-core/src/memory/contradiction.rs:104`, un `matches!` con los mismos dos tipos). El escalamiento del detector y la guarda coinciden hoy por texto, no por código: un tipo protegido nuevo tendría que sumarse en los dos lugares.
2. **Un daemon anterior responde `422`, no `400`.** D4 dice que, con la declaración, un daemon anterior responde `400` nombrando el campo. `UpdateRequest` rechaza el campo desconocido en el extractor `Json` de axum, que responde `422` (`a_misspelt_synthesis_field_is_refused` fija ese código para otro cuerpo estricto, y `an_invalid_patch_changes_nothing` acepta `400 | 422`). `specs/update` R4 dice `422`.
3. **El orden de los errores.** La guarda corre después de leer las dos filas y de `check_pair` (`dispute.rs:355-367`), así que un par no enlazado sigue siendo «no están en disputa»; `AnswerRequired` se valida antes de abrir la transacción (`dispute.rs:342-346`), así que una declaración sin `reason` es `400` aun sobre una fila que no existe. `specs/disputes` R3 lo dice.
4. **La marca va en todo veredicto declarado.** La spec (R2) la cuenta con `supersedes`; el código marca `user answered: ` el `reason` de cualquier veredicto con la declaración, también `conflicts`, `compatible` y `scoped`, como dice D2. El texto consolidado lo dice así.
5. **La apertura queda a 20 caracteres del tope.** La frase de ACTIVE CONFLICTS sumó 80: mide 2.028 de 2.048 (`the_instructions_fit_older_clients`). `specs/mcp` lo dice, porque una oración más en la apertura obliga a acortar otra.
