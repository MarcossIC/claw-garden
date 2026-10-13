# visibility-policy — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 y 4 | M (partes A + B; C y D siguen en la [exploración](exploration.md)) | project-identity y eval-golden-expansion (archivados) | verify |

## Problema

Detalle verificado en la [exploración](exploration.md). Lo que decide esta propuesta:

1. **La identidad del agente es frágil.** Sale sólo de `clientInfo.name`, con fallback a `"mcp"` (`crates/clawg-cli/src/mcp/server.rs:48, 223-227`). El usuario no tiene forma de fijarla. Con la spec MCP 2026-07-28 `clientInfo` pasa a ser opcional: un cliente que deje de mandarlo cae a `"mcp"` y todas sus memorias dejan de verse, sin ningún error. Tampoco hay forma de saber con qué identidad está guardando un agente.
2. **Las operaciones que modifican no tienen scope.** Search y recall filtran por agente (`crates/clawg-core/src/memory/search.rs:115-122`), pero el puente MCP lista, borra y resuelve sin mirar quién llama (`server.rs:446-496`), y el daemon tampoco (`crates/clawg-cli/src/daemon/api.rs:741-833`). **Cualquier agente puede borrar o reemplazar cualquier memoria** conociendo su id, y `list_memories` le muestra a un agente las filas de todos los demás.
3. **El nombre del agente llega crudo a una ruta.** El workspace se escribe en `<workspaces>/<agent_id>/<proyecto>/workspace.md` (`crates/clawg-core/src/memory/workspace.rs:42-47`). Un `clientInfo.name` como `../x` o `a/b` escribe fuera de su carpeta. Severidad baja (es el mismo usuario), pero la identidad deja de ser sólo del cliente: ahora también la escribe el usuario.

## Partición

La exploración ya proponía partir el cambio. Queda así:

| Parte | Qué | Dónde |
|---|---|---|
| **A. Identidad** | `clawg mcp --agent <nombre>`, prioridad flag > `clientInfo` > fallback, `memory_status` dice cuál se usa, segmento de ruta seguro | **este cambio** |
| **B. Scope de lo que modifica** | `list_memories`, `delete_memory` y `resolve_conflict` (y `PATCH`) sólo sobre las filas del agente que llama | **este cambio** |
| C. Compartir | política por agente, procedencia en `recall_context`/`search_memory`, `enum Scope` (columna o regla), pesos por scope | después de secret-hygiene |
| D. Prefiltro en vec0 | partition keys o metadata para el crowding | sólo si el golden set muestra crowding |

**Por qué C espera:** el mapa (`openspec/README.md`) exige que secret-hygiene cierre antes de abrir lo compartido. Compartir entre agentes hace viajar un secreto guardado por error al contexto de otro cliente y su proveedor, y hoy no hay enmascarado ni purga. A y B no comparten nada: endurecen el aislamiento que ya existe.

## Decisiones

**De dónde sale la identidad (pregunta 1): de las dos, con prioridad flag > `clientInfo` > `"mcp"`.**

- El flag lo controla el usuario, en el registro MCP del host (`claude mcp add clawg -- clawg mcp --agent claude-code`). No depende de lo que el cliente declare, y sobrevive a un cliente que deje de mandar `clientInfo`.
- Sin flag, todo sigue igual que hoy: el mismo `clientInfo.name`, sin normalizar. Normalizar (mayúsculas, espacios) cambiaría el `agent_id` de filas existentes y las escondería; eso es un re-key y queda para C, junto con los alias.
- El valor del flag se recorta y no puede quedar vacío. No se restringen más caracteres: el segmento de ruta se protege aparte.
- `memory_status` informa el agente en uso y de dónde salió (flag, cliente o fallback). Así el usuario sabe qué nombre fijar para no perder lo que ya tiene.

**Scope de list/delete/update (pregunta 5): la frontera es el agente, igual que en search. El proyecto no.**

- El agente es hoy el límite de visibilidad: search nunca devuelve filas de otro agente. Lo que modifica sigue la misma regla.
- El proyecto no es un límite, es relevancia: search ya permite `all_projects` y las `user_preference` cruzan proyectos. `list_memories` sigue listando todos los proyectos del agente, como hoy.
- Una fila de otro agente se trata como inexistente: el mismo error "no existe" que un id que no está. Decir "existe pero no es tuya" le confirmaría a un agente que el id está ocupado.
- El daemon recibe el agente como parámetro opcional. Sin él, la operación no tiene scope: es la del dueño de la base (el visor `clawg memories`, `curl`, las herramientas de evaluación), que sigue viendo todo. El puente MCP lo manda siempre.

**Ruta del workspace: un nombre seguro queda igual, uno peligroso se codifica.**

- Un `agent_id` sin separadores, sin `.`/`..`, sin caracteres de control ni los que Windows no admite en un nombre de archivo, y sin `%`, es su propio segmento: la ruta de todos los workspaces que existen hoy no cambia, y la sección USER escrita a mano sigue donde estaba.
- Uno peligroso se codifica con `%XX` byte a byte. `%` cuenta como peligroso para que la codificación no choque con un nombre literal.

## Alcance

1. `clawg mcp --agent <nombre>` y la prioridad de la identidad.
2. `memory_status` con el agente en uso y su origen.
3. `list_memories`, `delete_memory` y `resolve_conflict` del puente MCP, sólo sobre filas del agente.
4. `agent` opcional en `DELETE` y `PATCH /v1/memories/{kind}/{id}` (query) y `agent_id` opcional en `POST /v1/disputes/resolve` (cuerpo). Presente, la fila tiene que ser de ese agente; si no, 404 sin cambiar nada.
5. Segmento de ruta seguro para el workspace.

## Fuera de alcance

- Compartir entre agentes, la política configurable, la procedencia y los pesos por scope (parte C).
- `enum Scope` (`crates/clawg-core/src/memory/record.rs:19`): se decide en C, con la pregunta 8 (columna o regla). Sigue en el backlog del mapa.
- Prefiltro en vec0 (parte D).
- Normalizar o unir identidades de agente (re-key de `agent_id`).
- `GET /v1/memories/{kind}/{id}` y `/v1/stats`: no tienen herramienta MCP; sólo los usa el visor del usuario.
- La actualización del protocolo MCP (mcp-protocol-upgrade): este cambio le da la identidad que necesita, no la usa.

## Éxito

`cargo test --workspace` sin fallas nuevas contra la base. Tests nuevos que fallan hoy:

- sin `clientInfo`, un puente con `--agent` lee y escribe las memorias de ese agente (regresión de identidad);
- `--agent` gana sobre `clientInfo`; sin flag, `clientInfo` gana sobre el fallback;
- `memory_status` nombra el agente y su origen;
- el agente B no puede borrar, reemplazar ni listar una memoria de A; A sí puede;
- con `agent` en el daemon, una fila ajena responde 404 y conserva su fila, su vector y sus disputas;
- un `agent_id` con separadores no escribe fuera de la carpeta de workspaces, y uno normal mantiene su ruta.

**rag-eval:** la búsqueda no cambia (ni `search.rs` ni el predicado), así que no hay métrica que comparar. Ver la verificación.

## Datos guardados

- **Esquema:** no cambia. No hay migración.
- **Identificadores:** ningún `agent_id` guardado cambia. El flag sólo decide con qué nombre lee y escribe el puente; quien lo configure con el mismo nombre que reportaba su cliente sigue viendo todo.
- **Riesgo de esconder memorias:** configurar `--agent` con un nombre distinto al que el cliente venía reportando hace que el puente deje de ver lo anterior, igual que hoy si el cliente cambia de nombre. No se pierde nada: las filas siguen en el visor y vuelven al quitar el flag o corregir el nombre. `memory_status` y `clawg memories stats` (por agente) muestran qué nombre usar.
- **Workspaces:** la ruta de un `agent_id` seguro no cambia. Uno inseguro pasa a una ruta codificada: su `workspace.md` anterior (fuera de la carpeta, o en una subcarpeta) no se mueve ni se borra; la sección AUTO se regenera sola en el próximo `recall_context`.
- **Test con datos de la versión anterior:** se siembran filas con el `agent_id` que reportaba un cliente y un `workspace.md` con sección USER en su ruta actual; con `--agent` igual a ese nombre y sin `clientInfo`, las filas se encuentran y la sección USER se conserva.
- **Rollback:** revertir los commits. Nada en la base depende de este cambio; quitar `--agent` del registro MCP vuelve a la identidad por `clientInfo`.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| El usuario configura `--agent` con otro nombre y "pierde" sus memorias | Media | No se pierden: siguen en el visor. `memory_status` dice el nombre en uso, y la documentación indica usar el que muestra `clawg memories stats` |
| Un puente nuevo habla con un daemon viejo, que ignora el `agent` y borra sin scope | Baja | Es el comportamiento de hoy, no una regresión. El puente arranca el daemon del mismo binario; se corrige al reiniciar el daemon |
| Un agente pierde una herramienta que usaba sobre filas de otro (borrar lo de otro agente) | Baja | Es justo lo que se cierra. El usuario sigue pudiendo por HTTP sin `agent`, y el agente dueño de la fila sigue pudiendo |
