# preference-model — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 2 y 4 | M | session-start-context (archivado el 2026-11-04) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md), verificado otra vez contra `develop` (`4242171`).

- **Las preferencias entran por agente, no por proyecto.** La consulta de las preferencias del workspace filtra `agent_id = ?1` y no mira `project_id` (`fetch_auto_rows`, `crates/clawg-core/src/memory/workspace.rs:351`; la consulta en `:376-385`). El comentario de `:364-370` dice que es a propósito. En un store real, 18 filas mezclan reglas generales con reglas de un solo repositorio, y todas entran en cada proyecto.
- **No mira quién lo dijo.** La misma consulta no lee `created_by_type` ni `source`. Una `user_preference` con importancia 8 o más que el agente guardó sin `user_stated`, inferida o sacada de un texto que leyó, entra bajo `## User Preferences (always active)` (`render_rows`, `workspace.rs:595`) en cada sesión y en cada proyecto, antes de cualquier consulta. Un save declarado queda `explicit` / `user` y uno sin declarar `explicit` / `agent` (`seal`, `crates/clawg-core/src/memory/provenance.rs:15-34`).
- **Sólo las del agente.** El mismo `agent_id = ?1` deja a Cursor sin las preferencias que la persona le dijo a Claude Code, aunque describen a la misma persona y el alcance de `sharing.json` deja leerlas ([`specs/sharing`](../../specs/sharing/spec.md), R5).
- **No hay cómo guardar una preferencia general desde un repositorio.** `save_memory` toma `project` o `project_path` y, sin ninguno, el proyecto del directorio del puente (`save_memory_impl`, `crates/clawg-cli/src/mcp/server.rs:1063-1075`); el comentario de `:1067` dice que un save nunca es «de todos los proyectos». El daemon ya acepta un save sin `project_id` (`SaveRequest.project_id`, `crates/clawg-cli/src/daemon/api.rs:391`), y `update_memory` con `clear_project` saca después una fila de todo proyecto.
- **Un hit no dice su origen.** La línea de un hit en `recall_context` (`format_hit_line`, `crates/clawg-core/src/memory/prompt_builder.rs:851`) y en `search_memory` (`format_hits`, `crates/clawg-cli/src/mcp/tools.rs:192`) lleva tipo, tier, puntaje y autor. La marca de origen (`mark`, `provenance.rs:37`) sólo la muestran los pares y `synthesize` ([`specs/provenance`](../../specs/provenance/spec.md), P4 y P5).

## Objetivo que sirve

1, porque lo que entra solo a cada sesión tiene que ser lo que la persona dijo; 2, porque una regla de un repositorio no es una regla de todos; 4, porque las preferencias describen a la persona y no al agente que las oyó. Va antes de instalar los hooks en todos los proyectos ([ROADMAP](../../ROADMAP.md), «Cómo se ordena», punto 1).

## Decisiones

Las decisiones están cerradas en la exploración ([«Decisiones»](exploration.md#decisiones-cerradas)): el alcance A, que las preferencias crucen agentes y que sólo lo declarado entre solo. Esta propuesta no las reabre; dice cómo se aplican. La cuarta, cómo se clasifican las preferencias ya guardadas, es una revisión a mano con la persona y queda fuera de este cambio.

1. **El alcance es el proyecto de la fila.** Con un proyecto, una preferencia entra al workspace si su `project_id` es el del proyecto o es NULL. Sin proyecto, entran las de todos, como las decisiones y los skills de la misma función (`workspace.rs:400-437` y `:452-477`) y como la búsqueda sin proyecto. Una preferencia general se guarda sin proyecto (punto 4). No hay columna nueva.
2. **Las preferencias siguen el alcance del lector.** La consulta de las preferencias usa el predicado de alcance de la búsqueda (`reach_predicate!`, `crates/clawg-core/src/memory/search.rs:78`): las propias, las sin autor y las de cada autor que `sharing.json` no le oculta. `regenerate_auto_section` recibe ese alcance, y el daemon le pasa el mismo que resuelve para la búsqueda (`reach`, `api.rs:1557`). Las decisiones y los skills siguen siendo sólo del agente. La línea de una preferencia ajena es la misma que la de una propia, sin `by <autor>`: dice lo que declaró la persona, y `workspace.md` no guarda autores.
3. **Sólo entra sola una preferencia con `created_by_type = 'user'`.** Las filas guardadas antes del sello quedaron `explicit` / `user` (`specs/provenance`, P6), así que siguen entrando si cumplen lo demás. Lo inferido, una conclusión o lo importado con `agent` se alcanza buscando (punto 5). El umbral de importancia 8 y el tier 1 no cambian.
4. **`save_memory` toma `all_projects`.** Con `true`, la memoria se guarda sin proyecto, y la leen todos los proyectos. MUST NOT ir con `project`, `project_path` ni `new_project`, y el puente se niega sin llamar al daemon. Vale para cualquier tipo, como ya vale el save sin proyecto por HTTP; `protocol.md` dice que es para una preferencia que la persona tiene en todo lo que hace. El nombre es el de `search_memory` y `synthesize`, que el modelo ya conoce: «todos los proyectos».
5. **Lo inferido se alcanza buscando, marcado como tal.** Un hit de `user_preference` que tiene marca de origen la lleva después del tipo: `[AAAA-MM-DD #id user_preference inferred]` en `recall_context`, como el rótulo de ACTIVE CONFLICTS, y `N. #id [memory/user_preference/t1 inferred score=S]` en `search_memory`. La marca es la de `mark`: `inferred`, `imported` o `conclusion`. Sólo las preferencias: es la única fila que el modelo lee como una regla de la persona, y marcar cada hit cambiaría casi todas las líneas que lee. El core la pone en el hit (`RankedHit.origin`) y `POST /v1/search` la manda; el puente lee su ausencia, la de un daemon anterior, como sin marca.
6. **El protocolo dice qué entra solo.** `protocol.md` y `protocol.es.md` dicen, en «Type and importance» / «Tipo e importancia», que una `user_preference` declarada con 8 o más se imprime arriba de cada `recall_context` de su proyecto, o de todos si se guardó con `all_projects`, para cualquier agente, y que una búsqueda marca la que no declaró la persona. Son secciones fuera de la apertura, así que las instrucciones no cambian. Lo que cambia del contrato del agente es el argumento nuevo, en los dos idiomas.

## Alcance

1. `crates/clawg-core/src/memory/workspace.rs`: la consulta de las preferencias y la firma de `regenerate_auto_section`.
2. `crates/clawg-cli/src/daemon/api.rs`: el contexto pasa el alcance del lector a la regeneración.
3. `crates/clawg-core/src/memory/search.rs`: el origen de un hit de preferencia (`RankedHit.origin`) en las consultas de `memory`.
4. `crates/clawg-core/src/memory/prompt_builder.rs` y `crates/clawg-cli/src/mcp/tools.rs`: la marca en la línea del hit. `crates/clawg-cli/src/client.rs`: `Hit.origin`.
5. `crates/clawg-cli/src/mcp/server.rs` y `tools_es.rs`: `all_projects` de `save_memory`. `agent_contract.json` y `agent_contract.es.json`, bendecidos.
6. `protocol.md` y `protocol.es.md`.
7. Docs: `docs/reference/daemon.md`. `specs/sharing` (R5), `specs/retrieval`, `specs/provenance` y `specs/mcp` reciben los requisitos de la [spec](spec.md) al archivar.
8. Lo que el CI del pull request marque en Windows.

## Fuera de alcance

- Cómo decide el puente su proyecto por defecto (`ClawgMcp::pending`, `server.rs:529`).
- Cambiar filas ya guardadas: clasificar las preferencias que hoy entran, moverlas a «sin proyecto» o declararlas. Eso es la revisión a mano con la persona de la exploración, y `update_memory` con `clear_project` ya lo hace fila por fila.
- Qué alcanza la búsqueda: el predicado de proyecto sigue trayendo toda `user_preference` desde cualquier proyecto (`memory_project_predicate!`, `search.rs:109`), y RECENT del contexto sin consulta también. Es por eso que una preferencia que sale del workspace se sigue encontrando.
- Las decisiones y los skills del workspace, el umbral de 8, el tier, `AUTO_BYTE_CAP` y el recorte de session-start-context.
- La clave `topic=K` de una fila sin proyecto vista desde un proyecto: sigue sin mostrarse, como hoy.
- Una política de olvido para `memory` (exploración, «¿La tabla `memory` escala?»).
- Subir la versión y archivar.

## Cómo se mide el éxito

Primero, tests que fallan hoy. Corren en Linux con el embedder falso:

- Una preferencia inferida con importancia 9 no entra al workspace; una declarada, sí.
- Una preferencia de otro proyecto no entra; una sin proyecto, sí. Sin proyecto, entran todas.
- La preferencia declarada de otro agente entra; la de un autor que `sharing.json` oculta, no; una decisión de otro agente, tampoco. Por el daemon: el contexto sin consulta de `cursor` trae la preferencia que guardó `claude-code`, hasta que se la oculta.
- `save_memory` con `all_projects` guarda la fila sin proyecto, y se niega con `project` o `project_path`.
- Un hit de una preferencia inferida lleva `inferred` en `recall_context` y en `search_memory`; uno declarado, y uno de otro tipo, no.
- Filas sembradas como las escribe la versión anterior, una preferencia inferida y una de otro proyecto, salen del workspace y se siguen encontrando buscando desde el proyecto, con sus columnas intactas.

Lo que no puede cambiar:

- Una línea de hit que no es de una preferencia, o de una preferencia declarada, es byte a byte la de hoy.
- La suite pasa, y `cargo clippy --workspace --all-targets -- -D warnings` sale con 0 en Linux y, por el CI del pull request, en Windows.
- El contrato del agente cambia sólo en `all_projects` de `save_memory`, y en los dos idiomas.

Contra `prod`, pendiente en local ([verify](verify.md)): los bytes de `session-start` por proyecto antes y después, y cuántas filas son elegibles por alcance y por sello, contadas con una lectura.

`rag-eval --diff` no aplica: el ranking no cambia. La ingesta de evaluación sella cada fila como declarada (`user_stated: true`, `crates/clawg-core/src/eval_api.rs:735`), así que el workspace y las líneas de hit del agent view no deberían cambiar; con el golden real queda pendiente en local.

## Datos y vuelta atrás

No toca el esquema, los identificadores ni los estados, y no reescribe ninguna fila. Cambia qué filas entran solas al workspace y qué dice la línea de un hit. Una preferencia que deja de entrar se sigue encontrando con `search_memory` y `recall_context` desde cualquier proyecto, y se lee y se lista como antes: no se pierde ni se esconde. Un save con `all_projects` deja `project_id` NULL, como ya lo dejan `update_memory` con `clear_project` y un save por HTTP sin proyecto, y una versión anterior lo lee igual.

Un binario anterior abre la base sin cambios y vuelve a la conducta de antes. Un puente anterior contra este daemon ignora `origin`; este puente contra un daemon anterior no ve marcas, y su `all_projects` llega como un save sin proyecto, que ese daemon acepta. Para volver atrás alcanza con el binario anterior.
