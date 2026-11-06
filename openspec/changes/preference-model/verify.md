# preference-model — Verificación

## Suite y clippy

En Linux x86_64, build debug, toolchain 1.99.1, en `target`, sobre `b59ce4c`: el último commit de código, el arreglo que pidió el CI de Windows (abajo). Los commits de registro sólo tocan esta carpeta, el mapa de openspec y el ROADMAP. La misma suite y los mismos clippy pasaron antes sobre `1f8d375`.

- `cargo test --workspace --no-fail-fast` pasa, sin ningún test fallado. Los ignorados son los `#[ignore]` que ya tenía el repositorio: este cambio no agrega ninguno.
- `cargo clippy --workspace --all-targets -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy -p clawg-cli -- -D warnings`, lo que construye el release, también.
- Los contratos del agente pasan después de bendecirlos con `CLAWG_BLESS=1`, en los dos idiomas. El diff es sólo `all_projects` en el `inputSchema` de `save_memory`; las `instructions` no cambian, porque el protocolo cambió fuera de la apertura ([abajo](#el-contrato-del-agente)).
- En Windows, el CI de [#131](https://github.com/MarcossIC/claw-garden/pull/131) falló sobre `f23117e` en `the_protocol_says_which_preferences_open_every_session`: `no section Type and importance`. El checkout de Windows deja `protocol.md` con saltos `\r\n`, y el helper del test buscaba la sección con `\n`. Se reprodujo en Linux pasando los dos protocolos a CRLF, y `b59ce4c` hace que el helper lea el texto sin `\r`, como `opening`; pasa con CRLF y con LF. Sobre `b59ce4c`, los jobs `test (ubuntu-latest)` y `test (windows-latest)` pasaron. El cambio no toca código `cfg(windows)`.
- Los arreglos de la revisión de [#131](https://github.com/MarcossIC/claw-garden/pull/131) (PM8, PM9 y las lecturas acotadas del hook, [`specs/hooks`](../../specs/hooks/spec.md) HR4) pasan en Windows x86_64: `cargo test --workspace --no-fail-fast` y los dos clippy, con `--target-dir target-test`. Sus tests con un FIFO son sólo de Unix: en Windows sólo se comprobó que compilan, sacándoles el `cfg`. En Linux corrieron con la suite y los dos clippy en el CI de #132, que los llevó a `main` sobre `0e146ff9` y salió como 0.77.0: pasaron los jobs `test (ubuntu-latest)` y `test (windows-latest)`.

## Los tests en rojo

Cada test se corrió antes del código que lo hace pasar, salvo los dos que la tabla dice.

| Requisito | Test | Cómo fallaba |
|---|---|---|
| PM2 | `only_a_preference_the_user_stated_enters_the_workspace` | La preferencia `explicit` / `agent` entraba al workspace: `#1 is the agent's, reached by searching` |
| PM1 | `a_preference_of_another_project_stays_out_of_the_workspace` | La preferencia de P2 entraba al workspace de P1 |
| PM1 | `an_unscoped_workspace_takes_the_preferences_of_every_project` | Pasaba: fija lo que no cambia, el workspace sin proyecto |
| PM7 | `a_preference_out_of_the_workspace_is_still_found` | Escrito después del sello: la preferencia declarada de P2 seguía en el workspace de P1. La aserción de la marca se sumó con PM5, cuando el módulo de tests todavía no compilaba |
| PM3 | `the_workspace_takes_the_preferences_the_reader_reaches` | No compilaba: `regenerate_auto_section` no recibía el alcance |
| PM3 | `a_stated_preference_reaches_the_session_start_of_another_agent` | El contexto de `cursor` venía vacío (`"prompt":""`): la preferencia de `claude-code` no entraba |
| PM5 | `a_preference_hit_carries_its_origin`, `a_preference_hit_line_shows_its_origin`, `format_hits_shows_the_origin_of_a_preference`, `a_hit_from_an_old_daemon_has_no_origin` | No compilaban: el hit no tenía `origin` |
| PM5 | `a_recent_preference_shows_its_origin` | Escrito después del código de RECENT: con la consulta de RECENT dando `origin: None`, la línea de la preferencia inferida salía sin marca |
| PM5 | `a_marked_preference_is_one_segment` (`crates/clawg-eval/src/bin/rag-eval/agent_view/parse.rs`) | El parser del agent view no leía una línea marcada como hit: daba `[]` |
| PM4 | `a_save_for_every_project_has_no_project`, `all_projects_goes_without_a_project` | No compilaban: `save_memory` no tenía `all_projects` |
| PM6 | `the_protocol_says_which_preferences_open_every_session` | Con el protocolo de antes: `Type and importance names user_stated` |
| PM8 | `a_hit_without_a_project_shows_how_a_save_revises_it`, `a_marked_key_stays_before_the_author_of_the_hit`, `format_save_shows_the_key_only_where_a_save_revises_it`, `a_preference_of_every_project_is_replaced_with_all_projects_and_the_key_shown` | El hit y `read_memory` no mostraban la clave: `#68\ntabs` |
| PM8 | `the_protocol_says_where_a_key_is_shown` | El protocolo decía que una memoria sin proyecto no muestra clave |
| PM9 | `a_stated_save_seals_the_row_it_repeats_as_the_users`, `a_twin_restored_while_the_save_is_embedded_is_reinforced_not_copied` | La fila seguía `explicit` / `agent` después del save declarado |
| PM9 | `an_inferred_save_leaves_the_seal_of_the_row_it_repeats`, `a_stated_save_leaves_an_import_and_a_conclusion_as_they_are`, `an_inferred_save_leaves_the_importance_of_the_row_it_repeats` | Pasaban: fijan lo que no cambia |
| PM9 | `a_stated_save_raises_the_importance_of_the_row_it_repeats_and_never_lowers_it` | La fila seguía en importancia 5 y tier 3: `(5.0, 3)` en lugar de `(9.0, 1)` |
| PM9 | `a_preference_stated_after_it_was_inferred_enters_the_workspace` | Con la fila sellada pero en importancia 5, no entraba al workspace |

**Tests que cambiaron, y por qué:**

- `workspace_preferences_cross_project_boundaries` fijaba que la preferencia de otro proyecto entra; lo reemplaza `a_preference_of_another_project_stays_out_of_the_workspace`.
- `the_workspace_stays_own_and_conflicts_are_shared` (`prompt_builder.rs`) fija ahora con una decisión, y con el alcance que resuelve el daemon, que el workspace no mezcla lo que sigue siendo del agente; la preferencia ajena entra por PM3.
- `seed_preference` y el fixture de `a_context_without_a_query_skips_what_the_workspace_showed_and_the_dead` (`prompt_builder.rs`), y `context_takes_a_budget_and_clamps_it` (`crates/clawg-cli/tests/api.rs`), siembran preferencias declaradas: son las que el workspace imprime y lo que esos tests necesitan.
- `ranked_hit_serializes_to_json` (`crates/clawg-core/tests/public_api.rs`) suma que `origin` se omite sin marca y se manda con ella.
- `every_origin_a_row_can_show_is_known` (`parse.rs`) ata las marcas que el parser reconoce a las que da `provenance::mark`.
- `a_hit_without_a_project_shows_how_a_save_revises_it` reemplaza a `a_hit_without_a_project_shows_its_key_only_to_a_bridge_without_one`, que ya no es cierto para una preferencia (PM8).

## La spec contra el código

| Requisito | Dónde |
|---|---|
| PM1, PM2, PM3 | `AUTO_PREFERENCES_SQL` (`crates/clawg-core/src/memory/workspace.rs:358`): `created_by_type = 'user'`, `reach_predicate!` sobre `agent_id` y `(?2 IS NULL OR project_id = ?2 OR project_id IS NULL)`; `fetch_auto_rows` (`:373`) la liga con `reader_bind`; `regenerate_auto_section` (`:216`) recibe el `Reach`. Las consultas de decisiones y skills no cambian |
| PM3, el daemon | `context` resuelve el alcance una vez y lo pasa a la regeneración y al prompt (`crates/clawg-cli/src/daemon/api.rs:1576` y `:1582`) |
| PM4 | `SaveMemoryArgs.all_projects` (`crates/clawg-cli/src/mcp/server.rs:186`), la negativa (`:1066`) y el save sin proyecto (`:1088`); su descripción en español en `tools_es.rs:179` |
| PM5 | `preference_mark` (`crates/clawg-core/src/memory/provenance.rs:49`); `RankedHit.origin` (`search.rs:157`), leído en la KNN (`:614`), en el canal léxico (`:1244`) y en RECENT (`prompt_builder.rs:1123`); la línea en `format_hit_line` (`prompt_builder.rs:853`) y `format_hits` (`crates/clawg-cli/src/mcp/tools.rs:203`); `Hit.origin` (`crates/clawg-cli/src/client.rs:239`); el parser del agent view, `without_origin` (`parse.rs:184`) |
| PM6 | `protocol.md:100` y `:115`, `protocol.es.md:103` y `:118` |
| PM7 | Sin código propio: la búsqueda sigue trayendo toda `user_preference` desde cualquier proyecto (`memory_project_predicate!`, `search.rs`), y ninguna ruta reescribe filas |

## El contrato del agente

```diff
diff --git a/crates/clawg-cli/src/mcp/agent_contract.es.json b/crates/clawg-cli/src/mcp/agent_contract.es.json
--- a/crates/clawg-cli/src/mcp/agent_contract.es.json
+++ b/crates/clawg-cli/src/mcp/agent_contract.es.json
@@ -284,6 +284,14 @@
       "inputSchema": {
         "$schema": "https://json-schema.org/draft/2020-12/schema",
         "properties": {
+          "all_projects": {
+            "default": null,
+            "description": "True para una memoria que vale en todos los proyectos, como una preferencia que la persona tiene en todo lo que hace: se guarda sin proyecto, así la lee cada proyecto. No con project ni con project_path.",
+            "type": [
+              "boolean",
+              "null"
+            ]
+          },
           "content": {
             "description": "La memoria a guardar, escrita como una afirmación que se entiende sola: se va a leer en una sesión futura sin nada de esta conversación alrededor. Como máximo 64 KiB; parte un texto más largo en varios saves.",
             "type": "string"
diff --git a/crates/clawg-cli/src/mcp/agent_contract.json b/crates/clawg-cli/src/mcp/agent_contract.json
--- a/crates/clawg-cli/src/mcp/agent_contract.json
+++ b/crates/clawg-cli/src/mcp/agent_contract.json
@@ -284,6 +284,14 @@
       "inputSchema": {
         "$schema": "https://json-schema.org/draft/2020-12/schema",
         "properties": {
+          "all_projects": {
+            "default": null,
+            "description": "True for a memory that holds in every project, such as a preference\nthe user has wherever they work: it is saved without a project, so\nevery project reads it. Not with project or project_path.",
+            "type": [
+              "boolean",
+              "null"
+            ]
+          },
           "content": {
             "description": "The memory to store, written as a self-contained statement — it will be\nread back in a future session with none of this conversation around it.\nAt most 64 KiB; split a longer text into several saves.",
             "type": "string"
```

## `rag-eval`

- **`rag-eval --diff`.** No aplica a PM1 a PM8: el ranking no cambia. PM9 sí puede mover una fila, porque la importancia entra al puntaje, pero sólo cuando un save declarado repite un texto vivo con más importancia que la de su fila. El `--diff` del agent view ([punto 3](#medido-sobre-una-copia-de-prod)) no mueve ningún caso en `retrieval` ni en `similarity`; eso no descarta que PM9 mueva una fila de un store real, porque depende de esos saves.
- **El agent view con el golden real.** La ingesta de evaluación sella cada fila como declarada (`user_stated: true`, `crates/clawg-core/src/eval_api.rs:735`), así que ningún hit lleva marca y una preferencia sin proyecto o del proyecto del caso sigue entrando al workspace. Lo que podía moverse era una preferencia del golden guardada en otro proyecto que el del caso: no se movió ningún caso ([punto 3](#medido-sobre-una-copia-de-prod)).

## Medido sobre una copia de prod

Medido el 2026-11-06. Arrancar un daemon sobre el `~/.clawg` real puede migrar la base, y `POST /v1/context` reescribe `workspace.md`, así que cada medición empezó de una copia nueva del directorio de datos de `prod`: `memory.db`, leída por una conexión de sólo lectura y copiada con la API de backup de SQLite en un solo paso (una transacción de lectura, consistente aunque el daemon de `prod` tenga la base abierta), y `models/`, `workspaces/`, `sessions/` y `preferences.json`. Nunca `daemon.json`, `token`, el socket, los locks, `metrics.db` ni `backups/`. Las cinco copias tienen las mismas filas: 56.367 en `memory`, con la misma última fecha y las mismas 20 preferencias elegibles.

En Windows el directorio de datos sale de la carpeta del usuario, y sólo un build con la feature `testing` de clawg-core lo muda (`CLAWG_TEST_HOME`, `crates/clawg-core/src/paths.rs`). Los binarios son builds debug con esa feature, que sin `CLAWG_TEST_EMBEDDER` no cambia nada más: el modelo real se carga de la copia. El de la base es de `4242171`, en un worktree aparte; el de este cambio, de `1a00cc1f`, cuyo código de `clawg-core`, `clawg-cli` y `rag-eval` es el de 0.77.0. Cada daemon arrancó con `CLAWG_TEST_HOME` en su copia, el perfil `dev` y `--port 0`, registró la copia como su directorio de datos y contestó `/health` con su propio pid y el perfil `dev`.

1. **Las filas elegibles por alcance y por sello**, con esta lectura sobre la copia de `memory.db`:

   ```sql
   SELECT CASE WHEN project_id IS NULL THEN 'sin proyecto' ELSE project_id END AS alcance,
          created_by_type, source, agent_id, COUNT(*)
   FROM memory
   WHERE memory_type = 'user_preference' AND importance >= 8 AND tier = 1
     AND deleted_at IS NULL AND status <> 'superseded'
   GROUP BY 1, 2, 3, 4
   ORDER BY 1, 2, 3, 4;
   ```

   Dice cuántas preferencias de las que hoy entran dejan de entrar por el sello (`created_by_type` distinto de `user`) y cuántas por el proyecto, y cuántas ganan los otros agentes. Es el recuento que la exploración pidió antes del sello, y la entrada de la revisión a mano con la persona, que queda fuera de este cambio. P1 es el único proyecto con filas elegibles; P2 a P18 son los otros proyectos del store, de más a menos filas vivas.

   | Alcance | `created_by_type` | `source` | `agent_id` | Filas |
   |---|---|---|---|---|
   | P1 | `agent` | `explicit` | `claude-code` | 2 |
   | P1 | `user` | `explicit` | `claude-code` | 18 |

   Ninguna es sin proyecto ni de otro agente. De las 140 `user_preference` vivas, sólo estas 20 tienen importancia 8 o más y tier 1. En `4242171` la consulta filtraba sólo por agente, así que las 20 entraban al workspace de `claude-code` en cada proyecto. Con este cambio:
   - por el sello dejan de entrar 2, en todo proyecto;
   - por el proyecto, las 18 declaradas dejan de entrar en los otros 17 proyectos, y siguen entrando en P1 y en el workspace sin proyecto;
   - los otros agentes ganan las 18 en P1: sin `sharing.json`, cada agente alcanza las filas de `claude-code`, y ninguno tiene preferencias elegibles propias.
2. **Los bytes de `session-start` por proyecto, antes y después.** Para cada proyecto con filas, `POST /v1/context` sin `query` con `agent_id` `claude-code`, con un binario de `4242171`, la base de este cambio, y con uno de este cambio, cada uno sobre una copia nueva del mismo store: los bytes de `prompt` y los del bloque WORKSPACE. La [propuesta de session-start-context](../archive/2026-11-04-session-start-context/proposal.md#problema) midió un workspace de 14.560 bytes, 14.400 de ellos en 18 preferencias.

   El pedido es el de `clawg hook session-start`: `budget` 8.000 (`SESSION_BUDGET`, `crates/clawg-cli/src/hook.rs:52`) y `lang` `es`, la preferencia de la copia, para los 18 proyectos y sin proyecto. Con 8.000 el workspace sale recortado con los dos binarios, así que se repitió con 24.000, el máximo, para verlo entero. Cada binario y cada presupuesto corrió sobre su propia copia. Cada celda es antes (`4242171`) → después (este cambio). WORKSPACE son los bytes del bloque, desde `=== WORKSPACE ===` hasta el separador siguiente, con las preferencias que imprime entre paréntesis; — es que no hay bloque, y * que el prompt trae `=== BUDGET ===` porque el presupuesto dejó algo afuera.

   | Proyecto | `prompt`, 8.000 | WORKSPACE, 8.000 | `prompt`, 24.000 | WORKSPACE, 24.000 |
   |---|---|---|---|---|
   | P1 | 6.885* → 7.994* | 3.159 (3) → 3.159 (3) | 23.863* → 23.512* | 13.665 (16) → 13.326 (16) |
   | P2 | 7.522* → 7.494* | 3.159 (3) → — | 23.632* → 10.794 | 15.003 (18) → — |
   | P3 | 7.617* → 7.992* | 3.159 (3) → — | 23.171* → 10.333 | 15.003 (18) → — |
   | P4 | 7.612* → 7.337* | 3.159 (3) → — | 23.765 → 9.679 | 16.421 (20) → — |
   | P5 | 7.469* → 7.844* | 3.159 (3) → — | 23.950* → 10.185 | 15.936 (19) → — |
   | P6 | 7.724* → 7.871* | 3.159 (3) → — | 23.341* → 11.829 | 13.665 (16) → — |
   | P7 | 7.853* → 7.269* | 3.159 (3) → — | 23.407* → 10.569 | 15.003 (18) → — |
   | P8 | 7.629* → 7.988* | 3.159 (3) → — | 23.609* → 10.256 | 13.665 (16) → — |
   | P9 | 7.805* → 7.221* | 3.159 (3) → — | 23.359* → 10.521 | 15.003 (18) → — |
   | P10 | 7.664* → 7.811* | 3.159 (3) → — | 23.281* → 11.769 | 13.665 (16) → — |
   | P11 | 7.422* → 7.797* | 3.159 (3) → — | 23.903* → 10.138 | 15.936 (19) → — |
   | P12 | 7.674* → 7.090* | 3.159 (3) → — | 23.228* → 10.390 | 15.003 (18) → — |
   | P13 | 7.768* → 7.326* | 3.159 (3) → — | 22.176 → 8.090 | 16.421 (20) → — |
   | P14 | 7.768* → 7.326* | 3.159 (3) → — | 22.176 → 8.090 | 16.421 (20) → — |
   | P15 | 7.768* → 7.326* | 3.159 (3) → — | 22.176 → 8.090 | 16.421 (20) → — |
   | P16 | 7.768* → 7.326* | 3.159 (3) → — | 22.176 → 8.090 | 16.421 (20) → — |
   | P17 | 7.768* → 7.326* | 3.159 (3) → — | 22.176 → 8.090 | 16.421 (20) → — |
   | P18 | 7.768* → 7.326* | 3.159 (3) → — | 22.176 → 8.090 | 16.421 (20) → — |
   | sin proyecto | 7.809* → 7.795* | 3.159 (3) → 3.159 (3) | 23.520* → 23.238* | 11.943 (13) → 11.673 (13) |

   - En los 17 proyectos sin preferencias el bloque WORKSPACE desaparece. Con 8.000 ocupaba 3.159 bytes (3 preferencias, recortado), y RECENT toma ese lugar: pasa de 8 hits a 10 y el prompt queda cerca del tope. Con 24.000 el bloque ocupaba de 13.665 a 16.421 bytes (de 16 a 20 preferencias), y el prompt baja de 22.176–23.950 bytes a 8.090–11.829, sin recortar nada.
   - En P1 entra lo mismo con 8.000: 3.159 bytes, 3 preferencias. Con 24.000, 16 en los dos, de 20 elegibles antes y de 18 después, y el bloque baja de 13.665 a 13.326 bytes.
   - Sin proyecto el workspace toma las declaradas de todo proyecto: con 24.000, 13 en los dos, recortado.
   - Sin recorte, el workspace de la base mide hoy 16.421 bytes, con 20 preferencias.
3. **El agent view con el golden real**: `rag-eval --real --agent-view` contra la línea base de `openspec/config.yaml`, para ver que ningún caso se mueve.

   `rag-eval --real --agent-view --tag preference-model-tip`, con el `rag-eval` de este cambio y el modelo de una copia nueva, sobre el golden de 46 casos con el sha256 de la línea base (`363d3e08…`): `search_memory` 36/46 a 761,4 tokens de media y `recall_context` 37/46 a 1.037,3, lo mismo que la línea base (`docs/eval/results/2026-10-20-real-untrusted-recall-fix-agent-view.json`). `rag-eval --diff` contra ese archivo no mueve ningún caso, ni a mejor ni a peor, en `search.found`, `context.found`, los tokens de cada herramienta, `retrieval`, `similarity` y cada par de idiomas (`en>en` 7, `es>en` 19, `es>es` 20). El resultado queda en `docs/eval/results/2026-11-06-real-preference-model-tip-agent-view.json`, fuera de git.

## Pendiente en prod

- **Cursor real**, en el piloto del [ROADMAP](../../ROADMAP.md): una preferencia declarada a Claude Code entra al arranque de Cursor en el mismo proyecto.

## Veredicto

Sin CRITICAL en Linux ni en el CI de Windows. Lo que se medía contra `prod` se midió sobre copias ([arriba](#medido-sobre-una-copia-de-prod)), y el agent view no mueve ningún caso. Queda Cursor real, en el piloto ([«Pendiente en prod»](#pendiente-en-prod)).
