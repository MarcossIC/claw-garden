# preference-model — Verificación

## Suite y clippy

En Linux x86_64, build debug, toolchain 1.99.1, en `target`, sobre `b59ce4c`: el último commit de código, el arreglo que pidió el CI de Windows (abajo). Los commits de registro sólo tocan esta carpeta, el mapa de openspec y el ROADMAP. La misma suite y los mismos clippy pasaron antes sobre `1f8d375`.

- `cargo test --workspace --no-fail-fast` pasa, sin ningún test fallado. Los ignorados son los `#[ignore]` que ya tenía el repositorio: este cambio no agrega ninguno.
- `cargo clippy --workspace --all-targets -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy -p clawg-cli -- -D warnings`, lo que construye el release, también.
- Los contratos del agente pasan después de bendecirlos con `CLAWG_BLESS=1`, en los dos idiomas. El diff es sólo `all_projects` en el `inputSchema` de `save_memory`; las `instructions` no cambian, porque el protocolo cambió fuera de la apertura ([abajo](#el-contrato-del-agente)).
- En Windows, el CI de [#131](https://github.com/MarcossIC/claw-garden/pull/131) falló sobre `f23117e` en `the_protocol_says_which_preferences_open_every_session`: `no section Type and importance`. El checkout de Windows deja `protocol.md` con saltos `\r\n`, y el helper del test buscaba la sección con `\n`. Se reprodujo en Linux pasando los dos protocolos a CRLF, y `b59ce4c` hace que el helper lea el texto sin `\r`, como `opening`; pasa con CRLF y con LF. Sobre `b59ce4c`, los jobs `test (ubuntu-latest)` y `test (windows-latest)` pasaron. El cambio no toca código `cfg(windows)`.

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

## Lo que no se midió

- **`rag-eval --diff`.** No aplica a PM1 a PM8: el ranking no cambia. PM9 sí puede mover una fila, porque la importancia entra al puntaje, pero sólo cuando un save declarado repite un texto con más importancia que la de su fila. No se midió, porque necesita el modelo real y el golden fuera de git.
- **El agent view con el golden real.** Necesita el modelo real y el golden fuera de git (`docs/eval/`). La ingesta de evaluación sella cada fila como declarada (`user_stated: true`, `crates/clawg-core/src/eval_api.rs:735`), así que ningún hit lleva marca y una preferencia sin proyecto o del proyecto del caso sigue entrando al workspace. Lo que puede moverse es una preferencia del golden guardada en otro proyecto que el del caso: queda en [«Pendiente en local»](#pendiente-en-local).

## Pendiente en local

Lo que se mide contra `prod`. Necesita una copia aislada del directorio de datos de `prod` y un binario con este cambio, publicado o el build debug de esta rama. Los dos binarios corren sobre esa copia, abierta con `--profile dev` (un build debug no abre `prod`), y ninguno sobre el `~/.clawg` real, en el que arrancar un daemon puede migrar la base y `POST /v1/context` reescribe `workspace.md`. Cada medición empieza de una copia nueva, por lo mismo.

1. **Las filas elegibles por alcance y por sello**, con una lectura sobre una copia de `memory.db`:

   ```sql
   SELECT CASE WHEN project_id IS NULL THEN 'sin proyecto' ELSE project_id END AS alcance,
          created_by_type, source, agent_id, COUNT(*)
   FROM memory
   WHERE memory_type = 'user_preference' AND importance >= 8 AND tier = 1
     AND deleted_at IS NULL AND status <> 'superseded'
   GROUP BY 1, 2, 3, 4
   ORDER BY 1, 2, 3, 4;
   ```

   Dice cuántas preferencias de las que hoy entran dejan de entrar por el sello (`created_by_type` distinto de `user`) y cuántas por el proyecto, y cuántas ganan los otros agentes. Es el recuento que la exploración pidió antes del sello, y la entrada de la revisión a mano con la persona, que queda fuera de este cambio.
2. **Los bytes de `session-start` por proyecto, antes y después.** Para cada proyecto con filas, `POST /v1/context` sin `query` con `agent_id` `claude-code`, con un binario de `4242171`, la base de este cambio, y con uno de este cambio, cada uno sobre una copia nueva del mismo store: los bytes de `prompt` y los del bloque WORKSPACE. La [propuesta de session-start-context](../archive/2026-11-04-session-start-context/proposal.md#problema) midió un workspace de 14.560 bytes, 14.400 de ellos en 18 preferencias.
3. **El agent view con el golden real**: `rag-eval --real --agent-view` contra la línea base de `openspec/config.yaml`, para ver que ningún caso se mueve.
4. **Cursor real**, en el piloto del [ROADMAP](../../ROADMAP.md): una preferencia declarada a Claude Code entra al arranque de Cursor en el mismo proyecto.

## Veredicto

Sin CRITICAL en Linux ni en el CI de Windows. Queda como WARNING lo que se mide contra `prod` ([«Pendiente en local»](#pendiente-en-local)).
