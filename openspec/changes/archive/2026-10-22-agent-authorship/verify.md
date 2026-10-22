# agent-authorship — Verificación

## Linux (2026-10-22)

Sobre `44434e4`, en `target`, con el compilador de `rust-toolchain.toml`. El build no pudo bajar ONNX Runtime de `cdn.pyke.io` (la red lo rechazó), así que `ort-sys` se enlazó dinámicamente contra la misma versión, 1.24.2, sacada del wheel `onnxruntime` de PyPI (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1` y `LD_LIBRARY_PATH` a ese directorio). Ningún test usa el modelo real; el cambio no toca ese enlace.

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Cada commit de código de las [tareas](tasks.md) (`0472eb8`, `a3b1739`, `f800620`, `2231ec5`, `c0724ec`, `44434e4`) pasó la suite y clippy antes de commitearse.
- `the_agent_contract_matches_its_file` se bendijo con `CLAWG_BLESS=1` y el diff se revisó: cambian las descripciones de `delete_memory`, `restore_memory`, `update_memory`, `resolve_conflict` y `list_memories`, y el schema de `list_memories` suma `agent`. Las instrucciones del servidor no cambian.

## El binario real

Con `target/debug/clawg` de un build de tests, `CLAWG_TEST_HOME` en un directorio temporal, `CLAWG_TEST_EMBEDDER=fake` y `CLAWG_TEST_DEFAULT_PORT=off`, un `clawg serve --profile dev --port 0` sirvió por socket y TCP. Por HTTP, con dos agentes en el mismo proyecto:

- `agent-a` guardó un texto de 15 pedazos; `GET …/save?agent=agent-b` sobre su último pedazo devolvió los 15, sin faltantes;
- `PATCH ?agent=agent-b` le cambió la importancia (`200`);
- el mismo texto guardado por `agent-a` y después por `agent-b` dio `inserted` y `reinforced` con el mismo id;
- una `architectural_decision` con `topic_key` `storage/engine` de `agent-a`, y otra de `agent-b` con la misma clave: la respuesta de la segunda lista la primera en `replaced`;
- con `clawg sharing hide agent-b agent-a`, la lectura, el `PATCH` y el `DELETE` de `agent-b` respondieron `404` y `agent-a` siguió leyendo la fila (`200`); con `unhide`, `agent-b` volvió a leerla;
- `clawg doctor --profile dev` dio `ok` en `agents` y en las seis reglas de `memory`;
- `clawg stop --profile dev` salió con 0.

## `conflict-eval` y `revision-eval`

Con el doble determinista (`--fake`), sobre `develop` (`7473926`) y sobre esta rama:

- `conflict-eval --fake`: la misma salida, byte a byte salvo los tiempos.
- `revision-eval chains --fake`: el único cambio fijo es `m4.fixed.protocol_paragraph_tokens`, 101 → 134, por la oración que `protocol.md` suma sobre las claves compartidas. `m1.above` y las posiciones de `en-typescript-indent` en el store sin clave cambian de una corrida a otra con el mismo binario (tres corridas de esta rama dieron 1, 0 y 1, y `develop`, 1): son empates del doble, no del cambio. El conjunto de cadenas es de un solo agente, así que el resto no podía moverse.

`rag-eval` no aplica: la búsqueda y el ranking no cambian.

## RED antes de cada cambio

Sólo V20 se vio fallar antes del código: `published_migration_checksums_are_pinned` por la fila que faltaba, y los tres tests que cuentan las migraciones. En los demás commits los tests se escribieron junto con el código, en el mismo commit, y no se corrieron solos contra el código anterior. Lo que los ata al cambio: la mayoría reescribe un test que pasaba con la regla por agente afirmando lo contrario (`an_agent_cannot_delete_another_agents_memory` pasa a `another_agent_deletes_a_memory_unless_hidden`, `reinforce_if_duplicate_other_agents_row_is_not_reinforced` a `reinforce_if_duplicate_reinforces_another_agents_row`, `a_key_is_scoped_to_its_agent_and_project` a `a_key_is_scoped_to_its_project`, y así los de la [spec](spec.md)), y los que agregan una firma nueva (`Reader`, `open_pairs` con proyecto, `ListMemoriesArgs.agent`, `TopicVersions.agents`) no compilaban contra la anterior. Cada uno conserva, con `hide`, la aserción vieja.

## Lo que no se midió

- Windows no se corrió a mano: el CI del pull request corre la suite y clippy en Linux y Windows.
- El perfil dev con Claude Code, que pide autorización: dos puentes con `--agent` distintos, uno que guarda una nota larga y otro que la lee entera, la corrige, la borra y la restaura, y `clawg doctor` sobre una base de la versión anterior con dos titulares de una clave entre agentes. Lo cubren, sin el modelo ni el cliente reales, los tests de `server.rs` y `tests/api.rs` y la prueba con el binario de arriba.
- `conflict-eval` sobre un flujo de varios agentes: los umbrales del detector se midieron con candidatas de un solo agente ([`specs/disputes`](../../../specs/disputes/spec.md), «Umbral y banda del detector»). Conviene medirlo antes de history-import (44), que trae las notas de `migrated`.
- La precisión del aviso de topic-key-warning (68) cuando la fila que nombra es de otro agente: la mide `revision-eval warned`, que todavía no corrió sobre una base con varios agentes.
- Un escenario de `clawg update` sobre una base en V19: la migración sólo toca índices, y la vuelta atrás es la de toda migración (el snapshot `pre-migration`).
