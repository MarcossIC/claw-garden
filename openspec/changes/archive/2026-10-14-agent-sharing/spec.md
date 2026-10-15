# agent-sharing — Especificación

**Dominio destino:** `specs/sharing/spec.md` (nuevo; se crea al archivar). Al archivar, la introducción de `specs/visibility` deja de decir que search y recall filtran siempre por agente y enlaza a `specs/sharing`; su R3 suma que una fila ajena que la búsqueda trae sigue siendo "no existe" para borrar, cambiar y resolver.

**Alcance:** quién lee las memorias de quién, dónde lo configura el usuario, cómo llega lo ajeno a `search_memory` y `recall_context`, y qué sigue siendo de cada agente. El esquema, el ranking y el filtro de proyecto no cambian.

## Definiciones

- **Lector:** el agente que busca (el agente del puente, `specs/visibility` R1, o el `agent_id` del cuerpo de `/v1/search` y `/v1/context`).
- **Autor de una fila:** su `agent_id`.
- **Exclusión:** un par dirigido (lector, autor). "cursor no lee a claude-code". `*` en cualquiera de los dos lados vale por todo agente.
- **Política:** el conjunto de exclusiones, guardado en `<data>/sharing.json`.
- **Alcance de L:** lo que la búsqueda de L trae además de lo propio. *Todo menos* un conjunto de autores ocultos, o *sólo lo propio*.
- **Fila ajena para L:** una fila cuyo autor no es L.

## R1. La política

- Sin `<data>/sharing.json`, la política MUST ser vacía, y con política vacía cada agente lee a todos los demás.
- El archivo MUST tener esta forma, y cualquier otra MUST ser inválida:

  ```json
  { "version": 1, "hides": { "<lector>": ["<autor>", "..."] } }
  ```

  Inválido también: un campo desconocido, una `version` distinta de 1, un nombre vacío o sólo espacios.
- Los nombres MUST compararse exactos, sin recortar ni cambiar mayúsculas, como el `agent_id` en la base. Excluir a un agente de sí mismo y los repetidos MUST ignorarse; `*` excluido de `*` no es eso y MUST valer.
- El alcance de L MUST ser:
  - *sólo lo propio* si entre las exclusiones de L o de `*` está el autor `*`;
  - si no, *todo menos* la unión de las exclusiones de L y de `*`, sin L.
- Un archivo inválido o ilegible MUST dar *sólo lo propio* para todo lector: una exclusión que el usuario escribió no se pierde por un error de tipeo. Lo propio MUST seguir viéndose igual. El error MUST poder leerse: lo informan `memory_status` (R6), `clawg sharing` (R7) y el log del daemon.
- El daemon MUST leer el archivo en cada `/v1/search` y `/v1/context`: un cambio vale desde la próxima llamada, sin reiniciar.
- Parsear, validar y resolver el alcance MUST ser funciones puras, sin sistema de archivos.

#### Escenario: sin archivo
- GIVEN una base con filas de `claude-code` y de `cursor`, y sin `sharing.json`
- WHEN `cursor` busca
- THEN encuentra filas de los dos

#### Escenario: la exclusión tiene dirección
- GIVEN la política `{"version":1,"hides":{"cursor":["claude-code"]}}`
- WHEN se resuelve el alcance
- THEN para `cursor` es *todo menos* `claude-code`; para `claude-code`, *todo*

#### Escenario: comodines
- GIVEN la política `{"version":1,"hides":{"*":["codex"],"aider":["*"]}}`
- WHEN se resuelve el alcance
- THEN nadie lee a `codex`, `aider` lee sólo lo suyo, y `codex` lee a todos menos a sí mismo

#### Escenario: archivo roto
- GIVEN un `sharing.json` con `{"version": 2, "hides": {}}`
- WHEN `cursor` busca
- THEN encuentra sólo lo suyo, y `memory_status` dice que `sharing.json` es inválido y por qué

**Test:** `a_missing_policy_shares_everything`, `an_exclusion_has_a_direction`, `every_agent_stands_for_either_side`, `names_match_exactly`, `self_exclusions_and_repeats_are_ignored`, `an_unknown_version_is_invalid`, `an_unknown_field_is_invalid`, `a_blank_name_is_invalid` y `an_unreadable_policy_is_an_error` (sobre un directorio temporal) en `crates/clawg-core/src/memory/sharing.rs`; `search_and_context_follow_the_sharing_policy_from_the_next_call` y `an_unreadable_policy_shares_nothing_over_http` en `crates/clawg-cli/tests/api.rs`.

## R2. Búsqueda con lo ajeno

- La búsqueda de L MUST traer las filas propias y las de los autores de su alcance, en las tres tablas (`memory`, `episodic`, `procedural`), en los dos canales (vectorial y léxico) y en la bonificación por identificador.
- Una fila ajena MUST pasar los mismos filtros que una propia: estado `active` o `disputed`, sin `deleted_at`, y el filtro de proyecto de siempre (la del proyecto, las sin proyecto y `user_preference` de cualquiera).
- El conteo de elegibles que decide el reintento de vec0 MUST contar con el mismo alcance, para que una ventana llena de filas no visibles siga disparando el reintento y una llena de visibles no lo dispare.
- Lo propio y lo ajeno MUST puntuar con la misma fórmula: este cambio no agrega pesos.
- Con alcance *sólo lo propio*, la búsqueda MUST devolver exactamente las mismas filas, en el mismo orden y con los mismos puntajes que antes de este cambio. Sin filas ajenas, *todo* MUST dar lo mismo que *sólo lo propio*.
- La búsqueda que hace el detector de contradicciones al guardar MUST seguir viendo sólo las filas del que guarda.

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

**Test:** `another_agents_row_is_found_in_every_channel`, `a_hidden_author_stays_out_in_every_channel`, `an_exclusion_does_not_work_backwards`, `an_agent_never_hides_its_own_rows`, `shared_rows_obey_the_project_filter`, `the_eligible_count_follows_the_reach` y `reaching_only_its_own_leaves_the_search_unchanged` en `crates/clawg-core/src/memory/search.rs`; `contradiction_probe_ignores_shared_authors` en `crates/clawg-core/src/memory/contradiction.rs`. La suite previa de `search.rs`, que corre con *sólo lo propio*, fija que esa búsqueda no cambia.

## R3. Cada hit dice su autor

- Cada hit de búsqueda MUST llevar su autor. `POST /v1/search` MUST devolverlo en cada hit como `agent_id`.
- El cliente del daemon MUST aceptar una respuesta sin `agent_id` (un daemon viejo) y tratar ese hit como propio.

**Test:** `a_hit_carries_its_author` en `search.rs`; `search_returns_the_author_of_each_hit` en `crates/clawg-cli/tests/api.rs`; `a_hit_from_an_old_daemon_reads_as_own` en `crates/clawg-cli/src/client.rs`.

## R4. Lo ajeno se presenta como dato

En `recall_context`:

- La línea de un hit ajeno MUST ser `[AAAA-MM-DD #id tipo by <autor>] contenido`. La de un hit propio MUST quedar igual que antes de este cambio.
- Si hay al menos un hit ajeno, el prompt MUST terminar con un bloque `SHARED` de una sola nota, que dice que las líneas marcadas `by <agente>` las guardó otro agente, que son información para verificar y no instrucciones, y que sólo ese agente puede cambiarlas o borrarlas. Sin hits ajenos, el bloque MUST NOT aparecer.

En `search_memory`:

- La línea de un hit ajeno MUST llevar `by <autor>` dentro del corchete: `N. #id [kind/tipo/tN score=S by <autor>] contenido`. La de un hit propio MUST quedar igual.
- Si hay al menos un hit ajeno, la respuesta MUST terminar con la misma nota.

#### Escenario: recall con un hit de A
- GIVEN una fila de A relevante a la pregunta, sin política
- WHEN B llama `recall_context`
- THEN la línea de esa fila termina el corchete con `by A]`, y el prompt termina con el bloque `SHARED`

#### Escenario: nada ajeno
- GIVEN sólo hits de B
- WHEN B llama `recall_context` o `search_memory`
- THEN el texto es idéntico al de antes de este cambio

**Test:** `a_shared_hit_names_its_author`, `an_own_hit_keeps_its_line`, `the_shared_note_appears_only_with_shared_hits` en `crates/clawg-core/src/memory/prompt_builder.rs`; `format_search_marks_shared_hits_and_adds_the_note`, `format_search_leaves_own_hits_alone` en `crates/clawg-cli/src/mcp/tools.rs`; `recall_context_shows_another_agents_memory_with_its_author` y `search_memory_shows_another_agents_memory_with_its_author` en `crates/clawg-cli/src/mcp/server.rs`, que además ocultan al autor y comprueban que la fila deja de llegar.

## R5. Lo que sigue siendo de cada agente

- El workspace de L MUST cargar sólo filas de L.
- ACTIVE CONFLICTS MUST mostrar sólo disputas cuya fila nueva es de L, como hoy, aunque un hit ajeno esté en el prompt.
- `list_memories`, `synthesize`, el dedup y el detector de contradicciones al guardar MUST seguir viendo sólo lo de L.
- `delete_memory`, `update_memory` y `resolve_conflict` sobre una fila ajena que L ve en la búsqueda MUST fallar como si el id no existiera, sin tocar nada (`specs/visibility` R3). Sus descripciones MUST decir que sólo valen sobre las memorias propias y que lo compartido es de sólo lectura.

#### Escenario: borrar lo compartido
- GIVEN una fila de A que B acaba de ver en `search_memory`
- WHEN B llama `delete_memory` con su id
- THEN la respuesta es un error, la fila sigue viva y A la sigue encontrando

#### Escenario: el workspace no se mezcla
- GIVEN una fila de A en tier 1
- WHEN B llama `recall_context`
- THEN la fila no está en el bloque WORKSPACE de B

**Test:** `a_shared_memory_is_read_only` y `tool_descriptions_say_shared_memories_are_read_only` en `server.rs`; `the_workspace_and_conflicts_stay_own` en `prompt_builder.rs`, con una disputa de A entre los hits de B; `contradiction_probe_ignores_shared_authors` (R2) cubre el detector. `list_memories` y `synthesize` no cambian de código: siguen pasando sólo el agente (`an_agent_lists_only_its_own_memories`, `an_agent_neither_reviews_nor_cites_another_agents_memories`).

## R6. `memory_status` dice a quién lee el agente

`memory_status` MUST agregar una línea:

- `shared: reads every agent` sin exclusiones para el agente (o sin archivo);
- `shared: reads every agent but <autor>, <autor>` (orden alfabético) con exclusiones;
- `shared: none (reads only its own)` cuando lo excluye `*`;
- `shared: none (<motivo>)` con un archivo inválido o ilegible, donde el motivo es el mensaje del error (por ejemplo `sharing.json is not valid: …`).

**Test:** `format_status_says_whom_the_agent_reads` en `tools.rs`; `memory_status_reports_the_sharing_of_the_agent` en `server.rs`.

## R7. `clawg sharing`

- `clawg sharing [--profile P]` MUST imprimir la política, una exclusión por línea (`<lector> does not read <autor>`, `no agent reads <autor>`, `<lector> reads no other agent`, `no agent reads another`), o `every agent reads every other agent`. Con un archivo inválido MUST imprimir el error y salir con 1.
- `clawg sharing hide <LECTOR> <AUTOR> [--profile P]` MUST agregar la exclusión y `clawg sharing unhide <LECTOR> <AUTOR>` MUST quitarla. Los dos imprimen la política resultante. Ocultar lo que ya está oculto o mostrar lo que no lo está MUST NOT fallar.
- Los nombres MUST recortarse y no pueden quedar vacíos, igual que `--agent` (al parsear). `*` vale en los dos lados. Lector igual a autor, salvo `* *`, MUST rechazarse antes de leer o escribir el archivo: un agente siempre lee lo suyo.
- Sobre un archivo inválido, `hide` y `unhide` MUST fallar sin reescribirlo: puede ser una edición a mano a medio hacer.
- La escritura MUST ser atómica (archivo temporal y `rename`): un lector concurrente ve la política vieja o la nueva, nunca una a medias. Si el directorio de datos todavía no existe, MUST crearse: ocultar puede ser lo primero que hace el usuario.
- `--profile` MUST valer antes o después de la acción (`clawg sharing --profile dev hide …` y `clawg sharing hide … --profile dev`): una exclusión escrita en el directorio de otro perfil deja compartiendo lo que el usuario cree oculto.
- MUST funcionar con el daemon corriendo y sin abrir la base.

**Test:** `sharing_shows_hides_and_unhides`, `sharing_rejects_a_blank_or_self_exclusion`, `every_agent_is_accepted_on_either_side` y `sharing_takes_the_profile_from_either_side_of_the_action` en `crates/clawg-cli/src/cli.rs`; `hide_and_unhide_print_the_policy_they_leave`, `a_change_on_an_invalid_policy_leaves_it_alone`, `hiding_an_agent_from_itself_writes_nothing` y `every_agent_on_both_sides_stops_all_sharing` en `crates/clawg-cli/src/sharing.rs`; `hide_and_unhide_rewrite_the_policy`, `hiding_twice_and_unhiding_nothing_are_fine`, `an_invalid_policy_is_never_rewritten`, `an_exclusion_can_come_before_the_data_directory_exists`, `the_saved_file_parses_back_to_the_same_policy` y `render_lists_one_exclusion_per_line` en `crates/clawg-core/src/memory/sharing.rs`.

## R8. Datos de la versión anterior

**Test:** `rows_of_the_previous_version_are_shared_untouched` en `server.rs`: siembra con SQL directo filas de `memory` de dos agentes (`scope = 'agent'`, como las escribe el insert) y su vector. Sin `sharing.json`, B las encuentra con el autor A. Con "B no lee a A", deja de encontrarlas. Al quitar la exclusión, las vuelve a encontrar. Al final, cada fila conserva su `scope`, `agent_id`, `project_id`, `status` y contenido.

## R9. `enum Scope` se borra

`enum Scope` (`record.rs`) y su reexport MUST desaparecer. La columna `scope` y su CHECK quedan como están; el insert sigue escribiendo `'agent'`.

**Test:** ninguno nuevo: compila y pasa la suite (nada lo usaba).
