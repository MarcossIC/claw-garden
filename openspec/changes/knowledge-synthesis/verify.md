# knowledge-synthesis — Verificación

**Estado:** parcial. Los tests pasan en Linux, commit por commit. Falta lo que necesita la máquina de Marcos: Windows, el golden set y una prueba con el perfil dev (abajo, **Pendiente**).

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux, en cada commit de la rama.

| Commit | Pasan | Fallan |
|---|---|---|
| Base (`origin/main`, c01a0af, con visibility-policy) | 1.696 | 10 |
| `docs(openspec): propose knowledge synthesis driven by the agent` (5ccb77d) | 1.696 | 10 |
| `refactor(core): remove the unused LLM extraction pipeline` (675f574) | 1.563 | 10 |
| `feat(core): add tables for memory provenance and synthesis reviews` (8214a7d) | 1.564 | 10 |
| `feat(core): record which memories a saved memory was derived from` (b1e41d6) | 1.570 | 10 |
| `feat(core): show the sources of a derived memory` (71fa3a6) | 1.571 | 10 |
| `feat(core): list the memories no conclusion covers yet` (757f169) | 1.579 | 10 |
| `feat(daemon): accept memory sources and serve synthesis` (81bec0c) | 1.587 | 10 |
| `feat(mcp): let the agent draw conclusions from its memories` (631fccd) | 1.595 | 10 |
| `docs: list the synthesis endpoint and tool` (5b3ed48) | 1.595 | 10 |

Todos los commits compilan, y las 10 fallas son las mismas en todos.

Después se mergeó `main` (262f839, con mcp-protocol-upgrade y secret-hygiene) en la rama. Con el merge y `fix(core): drop a purged memory's provenance links` encima: **1.635 pasan, 10 fallan**, las mismas 10. El merge pidió dos ajustes:

- `synthesize` toma el agente del `RequestContext` de cada pedido, como el resto de las herramientas desde rmcp 3.5.1.
- `clawg purge` borra la fila de verdad, y SQLite puede darle su id a una fila nueva. Por eso ahora también borra sus enlaces de `memory_derivation` (spec R6). Vienen del entorno:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- 3 de `diff::tests` en `rag-eval`: leen `docs/eval/results/`, que está en `.gitignore` y no existe en un clon limpio.

**Por qué baja el conteo:** la limpieza borra 133 tests. Todos probaban el código que se borra: `extractor`, `miner`, `resolver`, `episodic_writer` y `models/error.rs`, más los lectores sin llamador de `admin.rs` (histograma de fallas y tendencia de pasos) y `update_trajectory_root_cause` y `fetch_prior_trajectories` de `store_insert.rs`. Ningún test de lectura de `episodic_trajectory` ni `procedural_skill` se borró: son los que cubren R1 (ver la spec), y pasan antes y después. El cambio agrega 32 tests nuevos.

Los warnings de compilación bajan de 11 a 6. Los 6 que quedan ya estaban en la base: `insert_row_with_embedding` en los tests del worker, `ceiling` y `production_lexical_budget` en `rag-eval`, entre otros.

### Windows

**Pendiente.** Correr `cargo test --workspace --target-dir <otro-dir>` en la máquina de Marcos. Se espera la línea de base de Windows sin fallas nuevas.

## Pendiente (máquina de Marcos)

1. **Windows:** la suite, como arriba.
2. **rag-eval:** `docs/eval/golden.json` no está en git. Compilar `rag-eval` en `origin/main` y en la rama, y correr:

   | Corrida | Qué se espera |
   |---|---|
   | `rag-eval --real --diff` | Diff vacío. La búsqueda no cambia: sigue consultando las tres tablas, y una conclusión es una fila de `memory` como cualquier otra. |
   | `rag-eval --real --agent-view --diff` | Diff vacío. El recordatorio va fuera de `build_memory_prompt` (diseño D6), así que el prompt medido no cambia. |

3. **Prueba manual con el perfil dev en Claude Code:**
   - guardar 20 o más memorias en un proyecto y ver que `recall_context` termina con la línea que nombra `synthesize`;
   - llamar `synthesize`, guardar una conclusión con `save_memory` y `derived_from`, y ver que `list_memories` la muestra con `from=#…`;
   - volver a pedir `recall_context` y ver que la línea ya no está;
   - abrir con el agente fijado a otro nombre (`clawg mcp --agent otro`) y ver que `synthesize` no muestra las memorias del primero;
   - leer las conclusiones que escribió el agente: la calidad no tiene métrica automática (propuesta, **Éxito**).

## Datos guardados

- La migración V10 es aditiva (dos tablas nuevas). La prueba `a_store_from_the_previous_version_opens_with_every_row_untouched` abre una base en V9 con una memoria, una trayectoria y un skill, y comprueba que ninguna fila cambia.
- Ninguna tabla se borra. Las filas que dejó la app de escritorio siguen alcanzables (R1).

## Solapamiento con otros cambios

- **visibility-policy** (mergeado antes que éste): `synthesize` y `derived_from` siguen su regla. Un agente sólo revisa y cita sus propias filas, y una fila ajena cuenta como inexistente. La revisión que apaga el recordatorio es por agente.
- **Migraciones:** visibility-policy no agregó ninguna, así que V10 queda libre. Si otro cambio publica una V10 antes, éste renumera su archivo y su checksum.
- **context-density:** el recordatorio es una línea fuera del prompt. Si ese cambio reescribe `recall_context`, tiene que conservarla.
