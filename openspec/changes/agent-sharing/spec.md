# agent-sharing — Especificación

**Dominio destino:** `specs/sharing/spec.md` (nuevo; se crea al archivar). Al archivar, la introducción de `specs/visibility` deja de decir que search y recall filtran siempre por agente y enlaza a `specs/sharing`; su R3 suma que una fila ajena visible por la política sigue siendo "no existe" para borrar, cambiar y resolver.

**Alcance:** quién puede leer las memorias de quién, dónde lo configura el usuario, cómo llega lo ajeno a `search_memory` y `recall_context`, y qué sigue siendo de cada agente. El esquema, el ranking y el filtro de proyecto no cambian.

## Definiciones

- **Lector:** el agente que busca (el agente del puente, `specs/visibility` R1, o el `agent_id` del cuerpo de `/v1/search` y `/v1/context`).
- **Autor de una fila:** su `agent_id`.
- **Permiso:** un par dirigido (lector, autor) con lector ≠ autor. "cursor lee a claude-code".
- **Política:** el conjunto de permisos, guardado en `<data>/sharing.json`.
- **Autores visibles para L:** L más todo autor con un permiso (L, autor).
- **Fila ajena para L:** una fila cuyo autor no es L.

## R1. La política

- Sin `<data>/sharing.json`, la política MUST ser vacía: cada agente ve sólo lo suyo, exactamente como antes de este cambio.
- El archivo MUST tener esta forma, y cualquier otra MUST ser inválida:

  ```json
  { "version": 1, "reads": { "<lector>": ["<autor>", "..."] } }
  ```

  Inválido también: un campo desconocido, una `version` distinta de 1, un nombre vacío o sólo espacios.
- Los nombres MUST compararse exactos, sin recortar ni cambiar mayúsculas, como el `agent_id` en la base. Un autor igual a su lector y los repetidos MUST ignorarse.
- Un archivo inválido o ilegible MUST dar política vacía (no se comparte nada) y MUST NOT cambiar lo que ve un agente sobre lo propio. El error MUST poder leerse: lo informan `memory_status` (R6), `clawg sharing` (R7) y el log del daemon.
- El daemon MUST leer el archivo en cada `/v1/search` y `/v1/context`: un cambio vale desde la próxima llamada, sin reiniciar.
- Parsear, validar y resolver los autores visibles MUST ser funciones puras, sin sistema de archivos.

#### Escenario: sin archivo
- GIVEN una base con filas de `claude-code` y de `cursor`, y sin `sharing.json`
- WHEN `cursor` busca
- THEN sólo encuentra filas de `cursor`

#### Escenario: el permiso tiene dirección
- GIVEN la política `{"version":1,"reads":{"cursor":["claude-code"]}}`
- WHEN se resuelven los autores visibles
- THEN para `cursor` son `cursor` y `claude-code`; para `claude-code`, sólo `claude-code`

#### Escenario: archivo roto
- GIVEN un `sharing.json` con `{"version": 2, "reads": {}}`
- WHEN `cursor` busca
- THEN encuentra sólo lo suyo, y `memory_status` dice que `sharing.json` es inválido y por qué

**Test:** `a_missing_policy_shares_nothing`, `a_grant_has_a_direction`, `names_match_exactly`, `self_grants_and_repeats_are_ignored`, `an_unknown_version_is_invalid`, `an_unknown_field_is_invalid`, `a_blank_name_is_invalid` en `crates/clawg-core/src/memory/sharing.rs`; `an_unreadable_policy_shares_nothing` en el mismo módulo, sobre un directorio temporal. Fallan hoy: el módulo no existe.

## R2. Búsqueda con lo concedido

- La búsqueda de L MUST traer filas de los autores visibles para L, en las tres tablas (`memory`, `episodic`, `procedural`), en los dos canales (vectorial y léxico) y en la bonificación por identificador.
- Una fila ajena MUST pasar los mismos filtros que una propia: estado `active` o `disputed`, sin `deleted_at`, y el filtro de proyecto de siempre (la del proyecto, las sin proyecto y `user_preference` de cualquiera).
- El conteo de elegibles que decide el reintento de vec0 MUST contar con los mismos autores, para que una ventana llena de filas no visibles siga disparando el reintento y una llena de visibles no lo dispare.
- Lo propio y lo ajeno MUST puntuar con la misma fórmula: este cambio no agrega pesos.
- Con política vacía, la búsqueda MUST devolver exactamente las mismas filas, en el mismo orden y con los mismos puntajes que antes de este cambio.
- La búsqueda que hace el detector de contradicciones al guardar MUST seguir viendo sólo las filas del que guarda.

#### Escenario: B lee a A
- GIVEN una fila de A "el deploy usa fly.io" y la política "B lee a A"
- WHEN B busca "dónde se hace el deploy"
- THEN la fila de A está entre los hits

#### Escenario: A no lee a B
- GIVEN la misma política y una fila de B
- WHEN A busca su contenido
- THEN la fila de B no aparece

#### Escenario: lo ajeno respeta el proyecto
- GIVEN "B lee a A" y una fila de A del proyecto P1
- WHEN B busca dentro del proyecto P2
- THEN la fila no aparece; si es `user_preference`, sí

**Test:** `a_granted_author_is_found_in_every_channel`, `a_grant_does_not_work_backwards`, `shared_rows_obey_the_project_filter`, `the_eligible_count_includes_granted_authors` y `without_grants_the_search_is_unchanged` en `crates/clawg-core/src/memory/search.rs`; `contradiction_probe_ignores_granted_authors` en `crates/clawg-core/src/memory/contradiction.rs`. Fallan hoy (salvo el de política vacía y el del detector, que fijan lo que no cambia): `SearchScope` no tiene autores concedidos.

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
- GIVEN "B lee a A" y una fila de A relevante a la pregunta
- WHEN B llama `recall_context`
- THEN la línea de esa fila termina el corchete con `by A]`, y el prompt termina con el bloque `SHARED`

#### Escenario: nada ajeno
- GIVEN la misma política, pero sólo hits de B
- WHEN B llama `recall_context` o `search_memory`
- THEN el texto es idéntico al de antes de este cambio

**Test:** `a_shared_hit_names_its_author`, `an_own_hit_keeps_its_line`, `the_shared_note_appears_only_with_shared_hits` en `crates/clawg-core/src/memory/prompt_builder.rs`; `format_search_marks_shared_hits_and_adds_the_note`, `format_search_leaves_own_hits_alone` en `crates/clawg-cli/src/mcp/tools.rs`; `recall_context_shows_a_granted_memory_with_its_author` y `search_memory_shows_a_granted_memory_with_its_author` en `crates/clawg-cli/src/mcp/server.rs`.

## R5. Lo que sigue siendo de cada agente

- El workspace de L MUST cargar sólo filas de L.
- ACTIVE CONFLICTS MUST mostrar sólo disputas cuya fila nueva es de L, como hoy, aunque un hit ajeno esté en el prompt.
- `list_memories`, `synthesize`, el dedup y el detector de contradicciones al guardar MUST seguir viendo sólo lo de L.
- `delete_memory`, `update_memory` y `resolve_conflict` sobre una fila ajena que L ve por la política MUST fallar como si el id no existiera, sin tocar nada (`specs/visibility` R3). Sus descripciones MUST decir que sólo valen sobre las memorias propias y que lo compartido es de sólo lectura.

#### Escenario: borrar lo compartido
- GIVEN "B lee a A" y una fila de A que B acaba de ver en `search_memory`
- WHEN B llama `delete_memory` con su id
- THEN la respuesta es un error, la fila sigue viva y A la sigue encontrando

#### Escenario: el workspace no se mezcla
- GIVEN "B lee a A" y una fila de A en tier 1
- WHEN B llama `recall_context`
- THEN la fila no está en el bloque WORKSPACE de B

**Test:** `a_shared_memory_is_read_only` y `the_workspace_and_conflicts_stay_own` en `server.rs`; `tool_descriptions_say_shared_memories_are_read_only` en `server.rs`.

## R6. `memory_status` dice de quién lee el agente

`memory_status` MUST agregar una línea:

- `shared: reads <autor>, <autor>` (orden alfabético) con permisos;
- `shared: none` sin permisos o sin archivo;
- `shared: none (sharing.json is invalid: <motivo>)` con un archivo inválido.

**Test:** `format_status_says_whom_the_agent_reads` en `tools.rs`; `memory_status_reports_the_sharing_of_the_agent` en `server.rs`.

## R7. `clawg sharing`

- `clawg sharing [--profile P]` MUST imprimir la política, un permiso por línea (`<lector> reads <autor>`), o `nothing is shared`. Con un archivo inválido MUST imprimir el error y salir con 1.
- `clawg sharing grant <LECTOR> <AUTOR> [--profile P]` MUST agregar el permiso y `clawg sharing revoke <LECTOR> <AUTOR>` MUST quitarlo. Los dos imprimen la política resultante. Conceder lo que ya existe o revocar lo que no existe MUST NOT fallar.
- Los nombres MUST recortarse y no pueden quedar vacíos, igual que `--agent`. Lector igual a autor MUST rechazarse al parsear: un agente siempre lee lo suyo.
- Sobre un archivo inválido, `grant` y `revoke` MUST fallar sin reescribirlo: puede ser una edición a mano a medio hacer.
- La escritura MUST ser atómica (archivo temporal y `rename`): un lector concurrente ve la política vieja o la nueva, nunca una a medias.
- MUST funcionar con el daemon corriendo y sin abrir la base.

**Test:** `sharing_shows_grants_and_revokes`, `sharing_rejects_a_blank_or_self_grant` en `crates/clawg-cli/src/cli.rs`; `grant_and_revoke_rewrite_the_policy`, `granting_twice_and_revoking_nothing_are_fine`, `an_invalid_policy_is_never_rewritten` y `render_lists_one_grant_per_line` en `crates/clawg-core/src/memory/sharing.rs`.

## R8. Datos de la versión anterior

**Test:** `rows_of_the_previous_version_are_shared_untouched` en `server.rs`: siembra con SQL directo filas de `memory` de dos agentes (`scope = 'agent'`, como las escribe el insert) y su vector. Sin `sharing.json`, B no las encuentra. Con "B lee a A", las encuentra con el autor A. Revocado, deja de encontrarlas. Al final, cada fila conserva su `scope`, `agent_id`, `project_id`, `status` y contenido.

## R9. `enum Scope` se borra

`enum Scope` (`record.rs`) y su reexport MUST desaparecer. La columna `scope` y su CHECK quedan como están; el insert sigue escribiendo `'agent'`.

**Test:** ninguno nuevo: compila y pasa la suite (nada lo usaba).
