# visibility-policy — Diseño

Las decisiones de producto (la partición, la prioridad de la identidad, el agente como frontera) están en la [propuesta](proposal.md). Acá van las de implementación de las partes A y B.

## D1. Dónde configura el usuario la identidad

| Opción | A favor | En contra |
|---|---|---|
| **a. Flag `clawg mcp --agent <nombre>`** | Vive en el registro MCP del host, que ya es por cliente: cada host lanza su propio puente. Sin archivos nuevos | Hay que editar el registro de cada host |
| b. Variable de entorno `CLAWG_AGENT` | Se puede poner en el `env` del registro | Se hereda: un `clawg mcp` lanzado desde una terminal con la variable puesta toma la identidad de otro |
| c. Archivo en el directorio de datos (`agents.toml`) | Es donde viviría la política de C | Mapea qué a qué: el puente no sabe qué host lo lanzó si no es por `clientInfo`, justo lo que no es confiable |

**Elegida a.** El archivo de política, si hace falta, es de la parte C, y no tiene un uso real todavía.

## D2. La identidad es una función pura

`resolve_agent(flag: Option<&str>, client: Option<&str>) -> AgentIdentity { id, source }` en `crates/clawg-cli/src/mcp/server.rs`, con `source` ∈ `Flag | Client | Fallback`. `agent_id_for` la llama con el `clientInfo.name` del peer. Se prueba sin `Peer`, que ningún test puede construir (`server.rs:17-22`).

El valor del flag se recorta en el parser de clap (`value_parser`), así un valor vacío se rechaza antes de arrancar y el puente nunca recibe uno.

## D3. El scope vive en el core, como parámetro

| Opción | A favor | En contra |
|---|---|---|
| **a. `owner: Option<&str>` en `delete_memory_core`, `set_importance_core` y `resolve_dispute_core`** | La comprobación está en la misma transacción que la escritura: nadie puede cambiar el dueño en el medio. Un solo lugar por operación | Cambia la firma de tres funciones y sus llamadores |
| b. El daemon lee la fila, compara y después llama a la función de hoy | No cambia el core | Dos viajes a la base y una ventana entre leer y escribir. Además, `delete_memory_core` borra el vector sin mirar la fila (`crates/clawg-core/src/memory/admin.rs:211-216`): un chequeo afuera que se olvide en un camino nuevo borra un vector ajeno |
| c. Funciones nuevas `*_owned` | Las viejas no cambian | Dos funciones casi iguales por operación |

**Elegida a.** `None` es la operación del dueño de la base, sin scope.

- **`set_importance_core`:** `AND (?4 IS NULL OR agent_id = ?4)` en el `UPDATE`. Ajeno = 0 filas, el mismo resultado que un id inexistente.
- **`delete_memory_core`:** el borrado del vector y la liberación de disputas corren hoy sin mirar el `UPDATE`, a propósito (barren lo que dejó una versión anterior). Con `owner`, antes de nada se busca si existe la fila con ese id y **otro** agente; si existe, la transacción termina sin escribir y devuelve 0. Si no existe, sigue como hoy: los barridos son no-ops sobre un id ausente.
- **`resolve_dispute_core`:** `read_pair_row` ya lee las dos filas dentro de la transacción; lee también `agent_id`, y una fila de otro agente es `ResolveError::NotFound`, igual que una inexistente, antes de `check_pair`.

## D4. Forma en HTTP

- **`DELETE` y `PATCH`:** `?agent=<id>` en la query, el mismo nombre que ya usa `GET /v1/memories` para filtrar. `PATCH` no lo lleva en el cuerpo porque el cuerpo es lo que se cambia.
- **`POST /v1/disputes/resolve`:** `agent_id` en el cuerpo, como `POST /v1/memories` y `/v1/search`.
- **Error:** `404` con el mismo texto que un id inexistente.
- **Daemon viejo con puente nuevo:** el `agent` de la query se ignora (esas rutas no tienen extractor de query) y el `agent_id` del cuerpo también (`ResolveRequest` no tiene `deny_unknown_fields`). La operación corre sin scope, que es lo de hoy.

## D5. Codificar el segmento del workspace

| Opción | A favor | En contra |
|---|---|---|
| **a. Codificar sólo lo peligroso, con `%XX`** | Los nombres de hoy (`claude-code`, `cursor`) no cambian de ruta, y la sección USER escrita a mano sigue donde está. Legible | Hay que tratar `%` como peligroso para que sea inyectiva |
| b. Rechazar el nombre | Simple | Un cliente con un nombre raro se queda sin workspace, y `clientInfo` no lo controla el usuario |
| c. Hash del nombre | Siempre seguro | Cambia la ruta de todos los workspaces que existen: la sección USER queda huérfana |

**Elegida a.** La función es pura (`workspace_segment(agent_id) -> Cow<str>`) y sólo la usa `workspace_path`. `adopt_legacy` lee las carpetas de agente de disco: ya son segmentos, así que arma la ruta con el nombre de la carpeta en lugar de pasar por `workspace_path`, para no codificar dos veces.

## D6. `list_memories` del puente

`ListArgs.agent` ya existe y el daemon ya filtra por agente (`crates/clawg-cli/src/daemon/api.rs:228`, `admin.rs:381-383`). El puente sólo lo completa con el agente que llama. El esquema de la herramienta no cambia: el agente no elige a quién listar.
