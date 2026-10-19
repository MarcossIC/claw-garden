# session-handoff — Diseño

## D1. El id sale del hook y lo recuerda el daemon

**Elegido:** los dos hooks mandan `{agent_id, project_id, session_id}` a `POST /v1/sessions` y el daemon guarda, en un `Mutex<HashMap<(String, Option<String>), String>>` dentro de `AppCtx`, el último id de cada par. `save_memory` lo lee al aceptar.

| Alternativa | Por qué no |
|---|---|
| `CLAUDE_CODE_SESSION_ID` en el entorno del puente | No está en la documentación oficial del host, y un proceso MCP se lanza una vez: después de `/clear` el id cambia y el del entorno no |
| `_meta` de la solicitud MCP | Sin evidencia de que el host lo mande |
| Un archivo que escribe el hook y lee el puente | Estado en disco con carreras entre sesiones, lo que 25 ya descartó |
| Que el modelo lo pase | No lo conoce, puede inventarlo, y suma un argumento a una herramienta que se describe en cada sesión |

`user-prompt-submit` también registra, con dos efectos: un daemon reiniciado vuelve a tener el id en el próximo prompt (sin esperar a otro `SessionStart`), y con dos sesiones del mismo agente en el mismo proyecto gana la que recibió el último prompt, que es la que más probablemente guarda. La ambigüedad que queda (dos sesiones trabajando a la vez sin prompts nuevos) se acepta: sólo afecta la atribución, nunca el contenido.

El registro no se persiste: un id viejo después de un reinicio sería peor que `NULL`. Tampoco vence: con hooks instalados se renueva en cada prompt.

El par lleva el `project_id` tal como lo resuelven el hook y el puente desde el directorio de trabajo (`ProjectIdentity::resolve`), así que un save del puente y el registro del hook coinciden sin que el daemon resuelva nada.

## D2. El sello va en el evento, antes del outbox

**Elegido:** `MemoryEvent::session_id: Option<String>` con `#[serde(default)]`, puesto por `save_memory` antes de `accept`; `NewMemoryRecord::source_session_id` lo copia y el `INSERT` lo escribe.

**Alternativa:** leer el registro en el worker al insertar. Descartada: una reanudación después de un kill corre en un daemon nuevo cuyo registro está vacío, o tiene otra sesión, y el save quedaría atribuido a la sesión equivocada. En el evento, el outbox guarda lo que valía al aceptar.

Un save que refuerza una fila existente (mismo texto) no cambia su sesión: la fila es la del primer save.

## D3. LAST SESSION, fuera de la competencia por relevancia

**Elegido:** una consulta propia, `SELECT … WHERE memory_type = 'session_summary' AND status IN ('active','disputed') AND deleted_at IS NULL AND <proyecto> ORDER BY created_at DESC, id ASC`, filtrada por autor con el mismo `Reach` de la búsqueda y cortada en la primera fila admitida. El orden `id ASC` dentro de un mismo instante da el primer pedazo de un save largo, como en RECENT; `save_links` le pone la etiqueta `piece=1/N`.

- **Proyecto exacto, sin las filas sin proyecto:** un resumen sin proyecto no es «la sesión anterior de este proyecto». Sin proyecto pedido, sólo las filas sin proyecto.
- **Con el sharing, también sin consulta:** RECENT es sólo del agente porque lo elige la fecha y no una consulta. El resumen es lo contrario: lo que más sirve es justamente el de la sesión anterior aunque la haya hecho otro agente (relevo entre clientes en el mismo proyecto). Llega marcado `by <author>` y con la nota de SHARED, como un hit compartido.
- **Sin umbral de antigüedad:** la línea lleva su fecha y el agente la juzga; un umbral sería una constante sin medir que escondería el único resumen de un proyecto retomado después de meses.
- **Sin accesos:** se eligió por ser el último, no por ser relevante; contarlo en cada contexto lo promovería de tier por su edad.
- **Sin repetirlo:** si el resumen es también un hit (él u otro pedazo del mismo save), sale de RECENT/LONG-TERM. El workspace no hace falta mirarlo: sólo imprime preferencias, decisiones y skills (`crates/clawg-core/src/memory/workspace.rs:257`, `:284`, `:336`), nunca un `session_summary`.
- **Fuera de los conflictos:** no se suma a las filas cuyos pares se buscan; un resumen nunca entra en un par (`protocol.md:94`).

**Alternativa:** subir el puntaje del último resumen dentro de la búsqueda. Descartada: no llegaría sin consulta, depende del corte relativo y cambiaría el ranking que mide el golden set.

## D4. Lugar en el presupuesto

**Elegido:** el orden de corte pasa a ser LONG-TERM, RECENT, LAST SESSION, pares de conflicto. El bloque tiene un solo hit, así que se acorta con ` [truncated]` hasta que no queda texto y recién entonces se va, como el último hit de cualquier bloque (`fit_to_budget`, `prompt_builder.rs:378`).

Cede ante el workspace (que nunca se corta) y los conflictos activos, que piden una pregunta al usuario. Queda por encima de los hits porque es lo único que no se puede recuperar buscando: no se parece a ninguna consulta.

## D5. La evaluación

`rag-eval --agent-view` parsea las secciones del prompt (`crates/clawg-eval/src/bin/rag-eval/agent_view/parse.rs:84`) y sólo cuenta RECENT y LONG-TERM. Un `session_summary` esperado que pase a LAST SESSION dejaría de contar como encontrado aunque esté en el prompt, así que LAST SESSION se parsea como un bloque de hits más.

## Probar a mano

Con la configuración de hooks de agent-lifecycle-hooks (25), sobre el perfil dev:

1. Abrir una sesión, trabajar y guardar un `session_summary`.
2. `clawg memories show <id> --profile dev`: `session` trae el id de la sesión del host.
3. Abrir otra sesión en el mismo proyecto: el contexto inyectado abre con `=== LAST SESSION ===` y ese resumen.
