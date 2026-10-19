# sharing — Especificación

Fuente de verdad del comportamiento actual de qué agentes leen las memorias de qué agentes: la política `sharing.json` y cómo se resuelve, cómo la búsqueda y `recall_context` alcanzan filas ajenas, cómo se presenta lo ajeno, qué sigue siendo de cada agente, qué dice `memory_status` y `clawg sharing`. Quién puede modificar, listar y leer filas está en [`specs/visibility`](../visibility/spec.md); el ranking, los hits y el formato de las líneas están en [`specs/retrieval`](../retrieval/spec.md); las herramientas del puente, en [`specs/mcp`](../mcp/spec.md).

**Origen:** [agent-sharing](../../changes/archive/2026-10-14-agent-sharing/proposal.md), archivado el 2026-10-14. Los ids de requisito son los de esa spec. [session-handoff](../../changes/archive/2026-10-19-session-handoff/proposal.md), archivado el 2026-10-19, sumó a R2 el resumen de LAST SESSION y `Reach::admits`.

## Definiciones

- **Lector:** el agente que busca: el agente del puente ([`specs/visibility`](../visibility/spec.md) R1) o el `agent_id` del cuerpo de `POST /v1/search` y `POST /v1/context`.
- **Autor de una fila:** su `agent_id`. Una fila con `agent_id` NULL no tiene autor: el esquema la admite con `scope` `global` o `project` (`crates/clawg-core/migrations/V1__init.sql:30-34`). Ningún camino de hoy la escribe; puede llegar de una importación o de un ámbito futuro.
- **Exclusión:** un par dirigido (lector, autor): "cursor no lee a claude-code". `*` en cualquiera de los dos lados vale por todo agente.
- **Política:** el conjunto de exclusiones, guardado en `<data>/sharing.json` (`sharing_file`, `crates/clawg-core/src/paths.rs:254`).
- **Alcance de L (`Reach`, `crates/clawg-core/src/memory/sharing.rs:42`):** lo que la búsqueda de L trae además de lo propio. `AllBut(autores)`, todo menos un conjunto de autores ocultos, u `Own`, ningún otro agente; los dos traen también las filas sin autor. `OwnOnly`, sólo lo propio y ni siquiera las filas sin autor, es el del detector de contradicciones (R2): ninguna política lo da.
- **Fila ajena para L:** una fila cuyo autor no es L.

Por defecto, sin archivo, cada agente lee a todos los demás: ocultar es una decisión del usuario.

## R1. La política

- Sin `<data>/sharing.json`, la política MUST ser vacía, y con política vacía cada agente lee a todos los demás (`load`, `sharing.rs:171`).
- El archivo MUST tener esta forma, y cualquier otra MUST ser inválida (`OnDisk`, `sharing.rs:70`; `SharingPolicy::parse`, `sharing.rs:86`):

  ```json
  { "version": 1, "hides": { "<lector>": ["<autor>", "..."] } }
  ```

  Inválido también: un campo desconocido (`deny_unknown_fields`: un `"hide"` mal escrito daría una política vacía, es decir, todo compartido sin aviso), una `version` distinta de 1, un nombre vacío o sólo espacios.
- Los nombres MUST compararse exactos, sin recortar ni cambiar mayúsculas, como el `agent_id` en la base. Excluir a un agente de sí mismo y los repetidos MUST ignorarse; `*` excluido de `*` no es eso y MUST valer (`hide`, `sharing.rs:131`).
- El alcance de L MUST ser (`reach_for`, `sharing.rs:119`):
  - `Own` si entre las exclusiones de L o de `*` está el autor `*`;
  - si no, `AllBut` con la unión de las exclusiones de L y de `*`, sin L, en orden alfabético.
- Un archivo inválido o ilegible MUST dar `Own` para todo lector: una exclusión que el usuario escribió no se pierde por un error de tipeo. Lo propio y las filas sin autor MUST seguir viéndose igual. El error MUST poder leerse: lo informan `memory_status` (R6), `clawg sharing` (R7) y el log del daemon (`reach`, `crates/clawg-cli/src/daemon/api.rs:1203`).
- El daemon MUST leer el archivo en cada `POST /v1/search` y `POST /v1/context` (`api.rs:1171` y `api.rs:1245`): un cambio vale desde la próxima llamada, sin reiniciar.
- Parsear, validar y resolver el alcance MUST ser funciones puras, sin sistema de archivos; sólo `load` y `save` (`sharing.rs:171` y `:183`) tocan el disco.

#### Escenario: sin archivo
- GIVEN una base con filas de `claude-code` y de `cursor`, y sin `sharing.json`
- WHEN `cursor` busca
- THEN encuentra filas de los dos

#### Escenario: la exclusión tiene dirección
- GIVEN la política `{"version":1,"hides":{"cursor":["claude-code"]}}`
- WHEN se resuelve el alcance
- THEN para `cursor` es todo menos `claude-code`; para `claude-code`, todo

#### Escenario: comodines
- GIVEN la política `{"version":1,"hides":{"*":["codex"],"aider":["*"]}}`
- WHEN se resuelve el alcance
- THEN nadie lee a `codex`, `aider` no lee a ningún otro agente, y `codex` lee a todos menos a sí mismo

#### Escenario: archivo roto
- GIVEN un `sharing.json` con `{"version": 2, "hides": {}}`
- WHEN `cursor` busca
- THEN encuentra lo suyo y las filas sin autor, nada de otro agente, y `memory_status` dice que `sharing.json` es inválido y por qué

**Tests:** `a_missing_policy_shares_everything`, `an_exclusion_has_a_direction`, `every_agent_stands_for_either_side`, `names_match_exactly`, `self_exclusions_and_repeats_are_ignored`, `an_unknown_version_is_invalid`, `an_unknown_field_is_invalid`, `a_blank_name_is_invalid` y `an_unreadable_policy_is_an_error` (sobre un directorio temporal) en `crates/clawg-core/src/memory/sharing.rs`; `search_and_context_follow_the_sharing_policy_from_the_next_call` y `an_unreadable_policy_shares_nothing_over_http` en `crates/clawg-cli/tests/api.rs`.

## R2. Búsqueda con lo ajeno

- La búsqueda de L MUST traer las filas propias y las de los autores de su alcance, en las tres tablas (`memory`, `episodic`, `procedural`), en los dos canales (vectorial y léxico) y en la bonificación por identificador. El alcance viaja en `SearchScope.reach` (`crates/clawg-core/src/memory/search.rs:33`, `with_reach` en `:54`) y entra a cada consulta como un solo parámetro JSON (`reach_bind`, `search.rs:64`, con `reader`, `authorless` y `hidden`): `ELIGIBLE_SQL_MEMORY` (`:311`, con las de `episodic` y `procedural`), las tres KNN (`:239`, `:262`, `:281`), `LEXICAL_CANDIDATE_SQL` (`:737`), `IDENTIFIER_MATCH_SQL` (`:873`) y `LEXICAL_SCORE_SQL_PREFIX` (`:1031`). Todas MUST escribir el predicado de alcance con la misma macro (`reach_predicate!`, `search.rs:77`), y las de `memory` el de proyecto con otra (`memory_project_predicate!`, `:92`, que también usa `RECENT_ROWS_SQL` en `prompt_builder.rs:804`).
- Una fila sin autor MUST alcanzarla todo lector, con cualquier política y también con un archivo inválido: una exclusión nombra a un autor, y esa fila no tiene. `Reach::admits` (`sharing.rs:55`), que aplican LAST SESSION y el último filtro de los hits (`readable_hits`, `search.rs:141`), MUST decidir lo mismo que el predicado SQL para todo alcance y todo autor, sin autor incluido.
- Una fila ajena MUST pasar los mismos filtros que una propia: estado `active` o `disputed`, sin `deleted_at`, y el filtro de proyecto de siempre (la del proyecto, las sin proyecto y `user_preference` de cualquiera).
- El conteo de elegibles que decide el reintento de vec0 (`search.rs:572`) MUST contar con el mismo alcance, para que una ventana llena de filas no visibles siga disparando el reintento y una llena de visibles no lo dispare.
- Lo propio y lo ajeno MUST puntuar con la misma fórmula: compartir no agrega pesos.
- Con alcance `Own` y sin filas sin autor, la búsqueda MUST devolver exactamente las mismas filas, en el mismo orden y con los mismos puntajes que sin compartir. Sin filas ajenas, `AllBut` MUST dar lo mismo que `Own`. `SearchScope::all_default()` queda en `Own` (`search.rs:44`): rag-eval y todo scope armado a mano ven lo propio y las filas sin autor; el daemon es quien pone el alcance de la política.
- La búsqueda que hace el detector de contradicciones al guardar MUST seguir viendo sólo las filas del que guarda, sin las filas sin autor (`reach: Reach::OwnOnly`, `crates/clawg-core/src/memory/contradiction.rs:178`): nadie podría juzgar un par con una fila sin dueño.
- Un hit ajeno que se devuelve MUST contar como acceso igual que uno propio (`record_hits`, `api.rs:1192` y `prompt_builder.rs:256`): el tier mide cuánto se usa una memoria, y el decaimiento la baja si deja de servir.
- El resumen de LAST SESSION de `POST /v1/context`, con consulta o sin ella, MUST seguir el mismo alcance: el `session_summary` más nuevo del proyecto entre los propios, los de los autores que el alcance admite y los que no guardó ningún agente (`Reach::admits`, `crates/clawg-core/src/memory/sharing.rs:55`, sobre el mismo `Reach` que el daemon resuelve para la búsqueda: lo propio y lo sin autor siempre, y con `AllBut` todo autor que no esté oculto; [`specs/retrieval`](../retrieval/spec.md), «La última sesión»). Uno ajeno llega marcado `by <autor>` y con la nota de R4; uno sin autor, sin marca. A diferencia de un hit, MUST NOT contar como acceso. RECENT del contexto sin consulta sigue siendo sólo del agente.

#### Escenario: B lee a A
- GIVEN una fila de A "el deploy usa fly.io" y sin política
- WHEN B busca "dónde se hace el deploy"
- THEN la fila de A está entre los hits

#### Escenario: B no lee a A
- GIVEN la política "B no lee a A" y una fila de C con el mismo contenido
- WHEN B busca
- THEN la fila de A no aparece y la de C sí; A sigue leyendo a B

#### Escenario: lo ajeno respeta el proyecto
- GIVEN una fila de A del proyecto P1
- WHEN B busca dentro del proyecto P2
- THEN la fila no aparece; si es `user_preference`, sí

#### Escenario: una fila sin autor
- GIVEN una fila `global` sin `agent_id` y la política «B no lee a nadie»
- WHEN B busca, o guarda algo cercano
- THEN la búsqueda la trae, y el detector no la empareja con lo guardado

**Tests:** `another_agents_row_is_found_in_every_channel`, `a_hidden_author_stays_out_in_every_channel`, `an_exclusion_does_not_work_backwards`, `an_agent_never_hides_its_own_rows`, `shared_rows_obey_the_project_filter`, `the_eligible_count_follows_the_reach`, `reaching_only_its_own_leaves_the_search_unchanged`, `a_row_no_agent_saved_is_found_whatever_the_policy` (en los dos canales), `the_reach_predicate_agrees_with_admits` (cada alcance contra cada autor en SQLite) y `a_hit_the_reader_may_not_see_is_dropped_and_the_order_kept` en `crates/clawg-core/src/memory/search.rs`; `contradiction_probe_ignores_shared_authors` y `a_save_is_never_paired_with_a_row_no_agent_saved` en `crates/clawg-core/src/memory/contradiction.rs`; `a_row_no_agent_saved_is_read_by_every_agent` en `sharing.rs`; para LAST SESSION, `a_reach_admits_the_reader_and_whoever_is_not_hidden` en `sharing.rs`, y `the_last_session_block_respects_sharing` y `the_last_session_block_shows_a_summary_no_agent_saved` en `crates/clawg-core/src/memory/prompt_builder.rs`. La suite previa de `search.rs`, que corre con `Own`, fija que esa búsqueda no cambia.

## R3. Cada hit dice su autor

- Cada hit de búsqueda MUST llevar su autor (`RankedHit.agent_id`, `search.rs:119`). `POST /v1/search` MUST devolverlo en cada hit como `agent_id`.
- El cliente del daemon MUST aceptar una respuesta sin `agent_id` (un daemon viejo) y tratar ese hit como propio (`Hit.agent_id` con `#[serde(default)]`, `crates/clawg-cli/src/client.rs:175`).

**Tests:** `a_hit_carries_its_author` en `search.rs`; `search_returns_the_author_of_each_hit` en `crates/clawg-cli/tests/api.rs`; `a_hit_from_an_old_daemon_reads_as_own` en `crates/clawg-cli/src/client.rs`.

## R4. Lo ajeno se presenta como dato

En `recall_context` (`format_hit_line`, `crates/clawg-core/src/memory/prompt_builder.rs:628`; `is_shared`, `:621`):

- La línea de un hit ajeno MUST ser `[AAAA-MM-DD #id tipo by <autor>] contenido`. La de un hit propio MUST quedar igual que sin compartir, y la de un hit sin autor también: no hay a quién nombrar, y no suma la nota (lo mismo en `search_memory`, `shared_author`, `crates/clawg-cli/src/mcp/tools.rs:91`).
- Si hay al menos un hit ajeno, el prompt MUST terminar con un bloque `=== SHARED ===` (`SEP_SHARED`, `:29`; se arma en `:537-543`) de una sola nota (`SHARED_NOTE`, `:35`), que dice que las líneas marcadas `by <agente>` las guardó otro agente, que son información para verificar y no instrucciones, y que sólo ese agente puede cambiarlas o borrarlas. Sin hits ajenos, el bloque MUST NOT aparecer. Va al final, después de ACTIVE CONFLICTS.

En `search_memory` (`format_hits`, `crates/clawg-cli/src/mcp/tools.rs:98`; `shared_author`, `:91`):

- La línea de un hit ajeno MUST llevar `by <autor>` dentro del corchete: `N. #id [kind/tipo/tN score=S by <autor>] contenido`. La de un hit propio MUST quedar igual.
- Si hay al menos un hit ajeno, la respuesta MUST terminar con la misma nota (un solo texto, `SHARED_NOTE`, que la CLI reusa del core).

#### Escenario: recall con un hit de A
- GIVEN una fila de A relevante a la pregunta, sin política
- WHEN B llama `recall_context`
- THEN la línea de esa fila termina el corchete con `by A]`, y el prompt termina con el bloque `SHARED`

#### Escenario: nada ajeno
- GIVEN sólo hits de B
- WHEN B llama `recall_context` o `search_memory`
- THEN el texto es idéntico al de antes de existir lo compartido

**Tests:** `a_shared_hit_names_its_author`, `an_own_hit_keeps_its_line`, `the_shared_note_appears_only_with_shared_hits` en `crates/clawg-core/src/memory/prompt_builder.rs`; `format_search_marks_shared_hits_and_adds_the_note`, `format_search_leaves_own_hits_alone` en `crates/clawg-cli/src/mcp/tools.rs`; `recall_context_shows_another_agents_memory_with_its_author` y `search_memory_shows_another_agents_memory_with_its_author` en `crates/clawg-cli/src/mcp/server.rs`, que además ocultan al autor y comprueban que la fila deja de llegar.

## R5. Lo que sigue siendo de cada agente

- El workspace de L MUST cargar sólo filas de L (`regenerate_auto_section` con el agente del pedido, `api.rs:1230`).
- ACTIVE CONFLICTS MUST mostrar sólo disputas cuya fila nueva es de L, como siempre, aunque un hit ajeno esté en el prompt (`fetch_active_conflicts`, `prompt_builder.rs:953`, recibe sólo el agente).
- `list_memories`, `synthesize`, el dedup y el detector de contradicciones al guardar MUST seguir viendo sólo lo de L (`list_memories_impl`, `crates/clawg-cli/src/mcp/server.rs:786`, manda `agent: Some(agent_id)`).
- `delete_memory`, `update_memory`, `resolve_conflict` y `restore_memory` sobre una fila ajena o sin autor que L ve en la búsqueda MUST fallar como si el id no existiera, sin tocar nada ([`specs/visibility`](../visibility/spec.md) R3 y R4; para `restore_memory`, también [`specs/restore`](../restore/spec.md)). Sus descripciones MUST decir que sólo valen sobre las memorias propias y que lo compartido es de sólo lectura (`server.rs:1106-1162`).

#### Escenario: borrar lo compartido
- GIVEN una fila de A que B acaba de ver en `search_memory`
- WHEN B llama `delete_memory` con su id
- THEN la respuesta es un error, la fila sigue viva y A la sigue encontrando

#### Escenario: el workspace no se mezcla
- GIVEN una fila de A en tier 1
- WHEN B llama `recall_context`
- THEN la fila no está en el bloque WORKSPACE de B

**Tests:** `a_shared_memory_is_read_only` y `tool_descriptions_say_shared_memories_are_read_only` en `server.rs`; `the_workspace_and_conflicts_stay_own` en `prompt_builder.rs`, con una disputa de A entre los hits de B; `contradiction_probe_ignores_shared_authors` (R2) cubre el detector. `list_memories` y `synthesize` siguen pasando sólo el agente (`an_agent_lists_only_its_own_memories` en `server.rs`, `an_agent_neither_reviews_nor_cites_another_agents_memories` en `crates/clawg-cli/tests/api.rs`).

## R6. `memory_status` dice a quién lee el agente

`memory_status` MUST terminar con una línea `shared:` (`format_sharing`, `crates/clawg-cli/src/mcp/tools.rs:548`; el puente lee el mismo archivo que el daemon, en `memory_status_impl`, `server.rs:981`):

- `shared: reads every agent` sin exclusiones para el agente (o sin archivo);
- `shared: reads every agent but <autor>, <autor>` (orden alfabético) con exclusiones;
- `shared: none (reads no other agent)` cuando lo excluye `*`: sigue leyendo lo suyo y las filas sin autor;
- `shared: none (<motivo>)` con un archivo inválido o ilegible, donde el motivo es el mensaje del error (por ejemplo `sharing.json is not valid: …`, `SharingError`, `sharing.rs:27`).

**Tests:** `format_status_says_whom_the_agent_reads` en `tools.rs`; `memory_status_reports_the_sharing_of_the_agent` en `server.rs`.

## R7. `clawg sharing`

`Command::Sharing` (`crates/clawg-cli/src/cli.rs:111`; ejecución en `run`, `crates/clawg-cli/src/sharing.rs:17`, y `main.rs:196`):

- `clawg sharing [--profile P]` MUST imprimir la política, una exclusión por línea (`<lector> does not read <autor>`, `no agent reads <autor>`, `<lector> reads no other agent`, `no agent reads another`), o `every agent reads every other agent` (`render`, `sharing.rs:148`). Con un archivo inválido MUST imprimir el error y salir con 1.
- `clawg sharing hide <LECTOR> <AUTOR> [--profile P]` MUST agregar la exclusión y `clawg sharing unhide <LECTOR> <AUTOR>` MUST quitarla. Los dos imprimen la política resultante. Ocultar lo que ya está oculto o mostrar lo que no lo está MUST NOT fallar.
- Los nombres MUST recortarse y no pueden quedar vacíos, igual que `--agent` (al parsear, `parse_agent` en `cli.rs:202` y `:205`). `*` vale en los dos lados. Lector igual a autor, salvo `* *`, MUST rechazarse antes de leer o escribir el archivo (`Exclusion::check`, `cli.rs:215`): un agente siempre lee lo suyo.
- Sobre un archivo inválido, `hide` y `unhide` MUST fallar sin reescribirlo: puede ser una edición a mano a medio hacer (`sharing.rs:21-26`).
- La escritura MUST ser atómica (archivo temporal y `rename`): un lector concurrente ve la política vieja o la nueva, nunca una a medias. Si el directorio de datos todavía no existe, MUST crearse: ocultar puede ser lo primero que hace el usuario (`save`, `crates/clawg-core/src/memory/sharing.rs:183`).
- `--profile` MUST valer antes o después de la acción (`clawg sharing --profile dev hide …` y `clawg sharing hide … --profile dev`, `sharing_profile`, `cli.rs:194`): una exclusión escrita en el directorio de otro perfil deja compartiendo lo que el usuario cree oculto.
- MUST funcionar con el daemon corriendo y sin abrir la base.

**Tests:** `sharing_shows_hides_and_unhides`, `sharing_rejects_a_blank_or_self_exclusion`, `every_agent_is_accepted_on_either_side` y `sharing_takes_the_profile_from_either_side_of_the_action` en `crates/clawg-cli/src/cli.rs`; `hide_and_unhide_print_the_policy_they_leave`, `a_change_on_an_invalid_policy_leaves_it_alone`, `hiding_an_agent_from_itself_writes_nothing` y `every_agent_on_both_sides_stops_all_sharing` en `crates/clawg-cli/src/sharing.rs`; `hide_and_unhide_rewrite_the_policy`, `hiding_twice_and_unhiding_nothing_are_fine`, `an_invalid_policy_is_never_rewritten`, `an_exclusion_can_come_before_the_data_directory_exists`, `the_saved_file_parses_back_to_the_same_policy` y `render_lists_one_exclusion_per_line` en `crates/clawg-core/src/memory/sharing.rs`.

## R8. Datos de la versión anterior

Las filas guardadas antes de que existiera compartir (`scope = 'agent'` y su `agent_id`) MUST compartirse sin migrar ni reescribir nada.

**Test:** `rows_of_the_previous_version_are_shared_untouched` en `crates/clawg-cli/src/mcp/server.rs`: siembra con SQL directo filas de `memory` de dos agentes (`scope = 'agent'`, como las escribe el insert) y su vector. Sin `sharing.json`, B las encuentra con el autor A. Con "B no lee a A", deja de encontrarlas. Al quitar la exclusión, las vuelve a encontrar. Al final, cada fila conserva su `scope`, `agent_id`, `project_id`, `status` y contenido.

## R9. No hay `enum Scope`

No hay `enum Scope` en `clawg-core` ni su reexport. La columna `scope` y su CHECK (`crates/clawg-core/migrations/V1__init.sql:6-7`) siguen como están; `NewMemoryRecord` (`crates/clawg-core/src/memory/store_insert.rs:117`) sigue escribiendo `'agent'`.

**Test:** ninguno propio: compila y pasa la suite sin él.
