# project-root-guard — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2, 3 | S | — (project-identity y project-indexing ya están archivados; project-clone-identity, 30, depende de éste) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código); lo que sigue está verificado otra vez contra `main` (`f8a361e`).

1. **El modelo decide qué directorio lee el daemon.** Cinco herramientas toman `project_path` (`crates/clawg-cli/src/mcp/server.rs:142`, `:160`, `:175`, `:189`, `:200`) y todas pasan por `project_for` (`server.rs:548-578`), que manda la raíz resuelta a `POST /v1/projects`. El daemon la acepta si no está vacía (`crates/clawg-cli/src/daemon/api.rs:1063-1065`), la guarda en `project` (`crates/clawg-core/src/memory/project.rs:80`) y lanza el indexado de sus docs y su código (`api.rs:1079`). Un `save_memory` con `project_path: "C:\\Users\\marco"` indexa la carpeta del usuario entera, y `search_project` sirve lo indexado (`api.rs:1152-1182`). Es el único camino por el que el daemon lee el disco por pedido del modelo, y esquiva los permisos de lectura del cliente.
2. **Una colisión de `project_id` mezcla dos proyectos en silencio.** Si la raíz da un id que ya es de otra raíz, el daemon responde 409 sin mover filas (`project.rs:84-87`, `:113`; `api.rs:1076`), pero el puente sólo deja un `log::warn!` y sigue con ese id (`server.rs:566-574`). Los saves de la segunda raíz caen bajo el id de la primera y `search_project` le sirve los docs de la primera (`root_of`, `crates/clawg-core/src/memory/project_docs/mod.rs:426-436`). El test `a_refused_registration_does_not_block_the_save` (`server.rs:1289-1310`) fija esa mezcla: espera la fila guardada. Después no hay forma de separarlas, porque ninguna fila guarda la raíz de la que salió.

## Decisiones

Defaults tomados donde la exploración dejaba la pregunta abierta. La primera se preguntó en el hilo con una tarjeta; mientras Marcos no elija otra, se sigue con la recomendada.

- **La regla vive en el daemon, con una marca en `project`** (tarjeta, opción recomendada). La columna `index_docs` dice si el daemon puede leer los archivos de esa raíz. Sin ella, el daemon no distingue una raíz registrada por un save de una que el usuario aceptó indexar, y la alternativa (que el puente no registre rutas ajenas) deja sin re-key las filas viejas guardadas bajo esas rutas ([diseño, D1](design.md#d1-dónde-vive-la-regla)).
- **Se indexa sin preguntar sólo la raíz del puente** (enfoque A de la exploración). Cualquier otra raíz necesita `clawg projects add <ruta>`, un comando del usuario que el modelo no puede mandar solo.
- **La guarda cubre sólo lo que lee el disco.** Las herramientas de memoria con otra ruta siguen funcionando: registran la raíz sin indexar (y con el re-key de siempre). El riesgo es la lectura, no la etiqueta; `all_projects` no cambia.
- **Las raíces ya registradas quedan aceptadas.** La migración pone la marca en 1 para todas: el usuario ya las eligió con su uso, y sus índices siguen sirviendo.
- **Una colisión es un error de la herramienta, en cada llamada** (enfoque A). El puente no guarda ni busca bajo un id que es de otra raíz, y lo vuelve a decir en cada llamada porque el estado no cambia solo. Los demás fallos de registro (daemon viejo, sin red) siguen sin bloquear: R6 de `specs/project` se acota a «salvo 409».
- **Una raíz que no es un directorio no se indexa.** Si se pide indexarla, el daemon responde 400; como etiqueta de memoria se sigue aceptando, igual que hoy.
- **No hay interruptor para apagar la guarda.** Se agrega si la fricción lo pide; sólo agranda lo permitido.

## Alcance

1. Migración `V13`: `project.index_docs`, 1 para las filas que ya existen.
2. `register_project` recibe si la raíz se indexa: la marca se guarda al crear la fila y sólo sube de 0 a 1, nunca baja.
3. `POST /v1/projects` toma `index_docs` (falso si falta); el refresco en segundo plano y `POST /v1/projects/docs/search` sólo corren sobre una raíz marcada; la búsqueda sobre una sin marcar responde 403 con el comando que la agrega.
4. El puente pide `index_docs` sólo para su propia raíz; `search_project` sobre otra devuelve el error del daemon tal cual.
5. El 409 pasa de warn a error de la herramienta, repetido en cada llamada.
6. `clawg projects add <ruta> [--profile P]`: registra e indexa la raíz de esa ruta por el daemon que está corriendo.
7. `.claude/CLAUDE.md` (comando, API, columna) y la descripción de `project_path` en `search_project`, con el contrato del agente bendecido.

## Fuera de alcance

- Ensanchar el hash, cambiar la identidad o re-keyar por la colisión (project-clone-identity, 30). Con este cambio, quien tenga dos carpetas `app` que chocan no puede usar memoria en la segunda hasta que 30 entregue un id más ancho o asignado por el registro (su W2 o W3).
- Separar filas ya mezcladas por una colisión previa (no hay dato para hacerlo).
- Un límite contra un agente con shell: puede leer el `token` y llamar al daemon directo, con `index_docs: true`. La guarda protege el camino MCP, como dice la exploración.
- `clawg projects list` y `remove`; permisos por agente sobre proyectos; `Host`/`Origin` y permisos del `token` (daemon-hardening, 38).
- Cambios de ranking o de recuperación.

## Datos guardados

Cambia el esquema: una columna nueva en `project`, con default, sin tocar ninguna otra tabla ni ningún `project_id`.

- **Migración:** `ALTER TABLE project ADD COLUMN index_docs INTEGER NOT NULL DEFAULT 1`. Abrir una base con la migración pendiente ya escribe un snapshot `pre-migration` en `<data>/backups` (`.claude/CLAUDE.md`, Data profiles).
- **Test de la versión anterior:** se siembra una base hasta `V12` con memorias, un proyecto registrado y su índice de docs; después de migrar, todas las filas son idénticas, el proyecto tiene `index_docs = 1` y su búsqueda de docs responde sin volver a pedir permiso.
- **Rollback:** una versión anterior de clawg rechaza una base con `V13` sin tocarla (como con cualquier migración nueva). El camino de vuelta es `clawg restore` del snapshot `pre-migration`, o a mano `ALTER TABLE project DROP COLUMN index_docs` y borrar la fila de la versión 13 de `refinery_schema_history` ([diseño, D2](design.md#d2-la-marca-y-su-migración)).

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- un `save_memory` con un `project_path` ajeno registra la raíz pero no indexa nada: `project_doc_file` sigue vacía;
- `search_project` sobre esa raíz falla con un mensaje que nombra `clawg projects add` y la ruta;
- `search_project` sobre la raíz del puente sigue indexando y respondiendo sin consentimiento;
- después de `clawg projects add`, la misma búsqueda indexa y responde;
- en una colisión, `save_memory` falla, nombra el id y la raíz que lo ocupa, y no escribe ninguna fila (hoy escribe 1); la llamada siguiente vuelve a fallar;
- un fallo de registro que no es 409 sigue sin bloquear el save;
- la base de la versión anterior abre con sus raíces aceptadas y nada cambiado.

No toca ranking ni retrieval: `rag-eval` no aplica (el conjunto dorado de `--docs` usa la raíz del puente). `verify.md` da el conteo de tests contra la base (1902 en Windows).
