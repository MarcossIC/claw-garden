# session-handoff — Verificación

## Linux (2026-10-19)

Sobre `2e2d13e` (el último commit de código), en `target`, con rustc 1.97.0:

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Cada commit de las [tareas](tasks.md) pasó la suite y clippy antes de commitearse.
- `the_agent_contract_matches_its_file` pasa sin bendecir: `save_memory` no ganó argumentos y la frase nueva de `protocol.md` va fuera de la apertura.

## CI (2026-10-19)

`.github/workflows/ci.yml` corre clippy y la suite en `ubuntu-latest` y `windows-latest`:

- En la cabeza de la rama (`05e14e6`, que suma a `2e2d13e` las docs y un test que tolera un hook que sale antes de leer su entrada), los dos jobs pasaron (run `39187873520`, el del pull request).
- En el push a `main` de ese merge (`cbb4994`) también pasaron (run `39188253043`). Ese merge no era para `main`: ver [Ramas](#ramas).

## Windows (2026-10-19)

Sobre `1f18565`, la cabeza de `develop`: el merge de este cambio (`51d04c0`) sobre client-setup, y después un commit que sólo toca `.github/workflows/release.yml` y `crates/clawg-cli/tests/conventions.rs`. En `target-test`, después de `cargo clean --workspace --target-dir target-test`:

- `cargo test --workspace --no-fail-fast --target-dir target-test`: pasa, ningún test falla. El único ignorado es `the_vectors_are_the_ones_fastembed_gave` (`crates/clawg-core/tests/e5_golden.rs`), que pide el modelo real y no es de este cambio. Corrieron todos los tests que nombran la [spec](spec.md) y las tareas.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: sale con 0.

Es la primera vez que este cambio y client-setup corren juntos: el CI de cada uno corrió sobre su propia rama.

## Lectura del código

| Requisito | Cómo quedó |
|---|---|
| R1 | `SessionRegistry` (`crates/clawg-cli/src/daemon/sessions.rs`) con `a_session_registry_keeps_the_latest_per_agent_and_project` y `a_session_id_is_checked`; `POST /v1/sessions` responde `204` o `400`, en `a_save_carries_the_registered_session` (`tests/api.rs`) |
| R2 | `save_memory` lee el registro antes de armar el evento y lo pone en `MemoryEvent::session_id`; `NewMemoryRecord::source_session_id` llega al `INSERT`. `a_save_writes_its_session_on_every_row` (`worker.rs`) cubre todos los pedazos de un save largo y un save sin sesión; `a_queued_save_keeps_its_session_in_the_outbox` lee el id en el evento del outbox; `an_event_without_a_session_reads_as_none` lee un evento anterior al campo; `rows_without_a_session_stay_reachable` lista y busca una fila sin sesión |
| R3 | `with_last_session` y `fetch_last_session` (`prompt_builder.rs`), en los dos armados; el autor se filtra con `Reach::admits`, el mismo criterio de la búsqueda. Tests: `the_last_session_block_shows_the_newest_summary` (otro proyecto, sin proyecto, borrado y más viejo quedan fuera; va antes de RECENT), `the_last_session_block_respects_sharing`, `the_last_session_block_is_not_repeated_as_a_hit`, `a_context_without_a_query_opens_with_the_last_session` (también uno de hace un mes) |
| R4 | `fit_to_budget` corta LONG-TERM, RECENT, LAST SESSION y recién después los conflictos; `the_last_session_block_gives_way_after_the_hits` comprueba además que el resumen no entra en `Fitted::shown`, así que no registra accesos |
| R5 | Los dos eventos de `clawg hook` registran en paralelo con lo que ya pedían e ignoran el resultado; `a_hook_registers_the_session_of_the_saves` (`tests/binary.rs`) corre el binario real: el save después de `session-start` lleva `s1` y el que sigue a un `user-prompt-submit` de otra sesión lleva `s2` |
| R6 | `context_segments` lee LAST SESSION; `the_last_session_block_counts_as_shown` |

## Medición con el modelo real (2026-10-19)

- **Qué necesita:** el modelo en el caché del perfil `dev` (`rag-eval --real --check-only` sale con 0), el golden set fuera de git (`docs/eval/golden.json`, sha256 `363d3e08854bfa222ccf9944508938e0e58a8dab9808696eca7a10525da0a1fb`, n = 46) y un build debug de `rag-eval` por brazo. `rag-eval` siembra un store temporal y no abre ningún store real.
- **Brazos:** el antes es `bca0d0a`, el `develop` anterior al merge de este cambio (primer padre de `51d04c0`), compilado en un worktree descartable con su propio directorio de build, borrados al terminar; el después, `1f18565` en `target-test`. Entre `51d04c0` y `1f18565` no cambia nada que `rag-eval` ejecute.
- **Comandos:** `rag-eval --real --agent-view --golden docs/eval/golden.json --tag session-handoff-base` y `… --tag session-handoff-tip`, con `--out` a un directorio por brazo; después `rag-eval --diff`. Los resultados quedaron en `docs/eval/results/2026-10-19-real-session-handoff-base-agent-view.json` y `…-tip-agent-view.json`, fuera de git.

| Métrica | Antes (`bca0d0a`) | Después (`1f18565`) |
|---|---|---|
| `recall_context`, `found` | 37/46 | 37/46 |
| `recall_context`, tokens media / máximo | 987,7 / 1.405 | 1.036,5 / 1.489 |
| `search_memory`, `found` | 36/46 | 36/46 |
| `search_memory`, tokens media / máximo | 761,4 / 1.206 | 761,4 / 1.206 |
| LAST SESSION, tokens por prompt | — | 87 en los 46 |

`--diff` no mueve ningún caso del ranking (`retrieval` y `similarity`), de `search.found` ni de `context.found`, tampoco en los pares de idioma (`en>en`, `es>en`, `es>es`). `context.tokens` empeora en los 46 casos, +48,7 de media con intervalo [+36,6, +60,9].

**El umbral de la [propuesta](proposal.md#cómo-se-mide-el-éxito) se cumple:**

- Ningún caso de `recall_context` se pierde.
- El aumento medio, 48,7 tokens (60,9 en el borde del intervalo), no pasa del tamaño medio de un `session_summary` del corpus: 64,2 tokens. Son los seis resúmenes del golden (c37 a c42, de 46 a 82 tokens), contados con el `tokenizer.json` del modelo, que es el que usa `rag-eval`. El mismo conteo da 7 tokens para `=== LAST SESSION ===`, que coincide con lo que mide `rag-eval`.

**De dónde salen los 48,7.** El eval siembra sin proyecto, así que todos los prompts abren con el mismo resumen, el más nuevo de los seis: 87 tokens con el encabezado. En 22 casos ese resumen ya estaba en RECENT. Se mudó a LAST SESSION y RECENT bajó 80 tokens, así que el neto es +7, el encabezado. En los otros 24 casos no estaba, y el neto es +87. Ningún caso esperaba ese resumen.

**La línea base de la propuesta no era la de hoy.** La propuesta comparaba contra la de `openspec/config.yaml` (`recall_context` 39/46 a 1.086,4). Desde search-abstention es 37/46 a 987,7: c09 y c10 sólo llegaban como filas de score 0 ([verify de search-abstention](../2026-10-18-search-abstention/verify.md)). El antes de esta medición da exactamente eso: contra `2026-10-18-real-search-abstention-agent-view.json` (`1307019`), `--diff` no mueve nada, ni siquiera los tokens. Así que la pérdida de esos dos casos es anterior a este cambio. `config.yaml` se actualiza al archivar.

## Claude Code sobre el perfil dev (2026-10-19)

Claude Code 2.1.303 como host, con el `clawg.exe` 0.24.0 de `target-test` compilado de `1f18565`. El store dev ya estaba en V16, así que no hubo migración. Configuración:

- **Hooks:** los de «Probar a mano» del [diseño](design.md#probar-a-mano), con la ruta absoluta del binario, pasados con `claude -p --settings <archivo>` para no tocar la configuración del usuario.
- **Puente:** por `--mcp-config` (`clawg mcp --profile dev --agent claude-code`, registrado como `clawg-verify`) con `--strict-mcp-config`.
- **Lectura:** cada hook se leyó con `--output-format stream-json --verbose --include-hook-events`.
- **Proyecto:** un directorio descartable con un solo `README.md`, para no mezclar filas ni indexar otro proyecto.

| Paso | Qué se vio |
|---|---|
| Sesión A, sin daemon dev | `SessionStart:startup` y `UserPromptSubmit` de clawg salen con 0 y sin salida. El puente arranca el daemon y el `session_summary` queda en `202` mientras carga el modelo; se guarda como #93. `clawg memories show 93 --profile dev` da `session:` igual al `session_id` que el host reportó en su `init`. El registro que alcanzó es el de `UserPromptSubmit`, que llegó con el daemon ya arriba |
| Sesión B | `SessionStart:startup` inyecta el sobre que abre con `=== LAST SESSION ===` y el resumen #93. #93 no se repite en RECENT, y no hay más filas. El segundo resumen se guarda como #94, con el `session_id` de B |
| Sesión C | El contexto inyectado abre con LAST SESSION = #94, y RECENT trae #93. `recall_context` con una consulta ajena («how is the frontend build pipeline configured with webpack») responde la frase de abstención (`2 searched, none close enough to show`) seguida de LAST SESSION con #94 |

Al terminar, #93 y #94 se borraron por `DELETE /v1/memories/memory/{id}?agent=claude-code` (siguen recuperables), y el daemon dev se paró con `clawg stop --profile dev`. Las filas que ya tenía el perfil no se tocaron. El proyecto descartable sigue registrado en la tabla `project` del perfil dev.

## Lo que encontró la verificación

### CRITICAL

Ninguno.

### WARNING

Ninguno.

### SUGGESTION

- **S1. La frase de abstención no nombra la última sesión.** Con `searched`, `hits = 0` y `withheld > 0`, `recall_context` dice `…; the workspace and any active conflicts are shown below.` (`retrieval_notice`, `crates/clawg-cli/src/mcp/tools.rs:363`), y ahora lo que sigue puede ser LAST SESSION, como en la sesión C. El bloque lleva su encabezado y se lee bien, pero la frase describe menos de lo que hay abajo. Cambiarla toca el texto que ve el agente y su test (`tools.rs:978`), así que queda para decidir aparte.
- **S2. `retrieval.hits` cuenta el save del resumen.** `Retrieval::Searched` y `Retrieval::Recent` se calculan antes de `with_last_session` (`prompt_builder.rs:225` y `:286`), así que un save que pasa de los hits a LAST SESSION sigue contando en `hits`. Lo que el puente decide con eso no cambia: la frase de abstención pide `hits = 0`, y en ese caso no hay hit que mudar. Queda dicho en [`specs/retrieval`](../../../specs/retrieval/spec.md).

## Ramas

El pull request de este cambio se abrió contra `main` por error y se mergeó (`cbb4994`, #45). En `main` se revirtió con `45f9d2f` (#46): el cambio entra a `main` desde `develop`, como todo lo que no es un hotfix. En `develop` está mergeado en `51d04c0`, y ahí se verificó y se archivó.

`main` lleva a la vez el merge y su revert. Por eso, el próximo merge de `develop` a `main` no trae el cambio: `git merge-tree --write-tree origin/main develop` da hoy un árbol sin `crates/clawg-cli/src/daemon/sessions.rs`. Hay que revertir `45f9d2f` en esa integración y comprobar que el archivo vuelva.

## Citas que el cambio corrió

El cambio sumó líneas a `prompt_builder.rs`, `daemon/api.rs`, `hook.rs`, `client.rs`, `sharing.rs`, `events.rs`, `store_insert.rs`, `eval_api.rs` y `serve.rs`. Al archivar se volvieron a verificar contra el código de `1f18565` las citas a esos archivos en todas las specs principales.

## Veredicto

Verificado en Linux, en el CI de Linux y Windows, en Windows sobre `develop`, con `rag-eval --real --agent-view` dentro del umbral de la propuesta y con Claude Code sobre el perfil dev, donde cada sesión nueva abrió con el resumen de la anterior y cada save llevó el `session_id` del host. Sin CRITICAL ni WARNING; las dos SUGGESTION quedan abiertas. Se archivó el 2026-10-19.
