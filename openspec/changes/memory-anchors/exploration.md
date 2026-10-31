# memory-anchors — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 (y 3) | M–L, por pasos (el paso 0 es S, sólo lee y no cambia código) | — para el paso 0; el índice de docs de [project-indexing](../archive/2026-10-15-project-indexing/proposal.md) (15, archivado) para los pasos 1 a 3; se coordina con [session-lifecycle](../session-lifecycle/exploration.md) (99), que lee el mismo contexto git del save, y con [agent-eval](../agent-eval/exploration.md) (96), que mide si el aviso cambia lo que hace el agente | exploración (borrador) |

**Tesis:** una memoria que habla de código envejece cuando el código cambia, y hoy clawg no lo puede saber. El protocolo pide escribir cada save «con las rutas y los identificadores exactos» (`crates/clawg-cli/src/mcp/protocol.md:14-16`) y dice que, si una memoria contradice al código, se compruebe contra el código (`protocol.md:148-155`), pero nada le avisa al agente **cuál** memoria nombra un archivo que cambió después de guardarla. La propuesta es anclar cada save a los archivos del proyecto que nombra (ruta y hash del contenido al guardarlo, más el commit y la rama de ese momento) y, al leerlo, marcar `changed=` cuando alguno ya no es el mismo. La marca es evidencia para el agente: no oculta, no borra y, al principio, no cambia ningún puntaje.

**De dónde sale.** De dos documentos de investigación previos, que no están en el repositorio: un diseño de integración con Claude Code («memoria anclada al índice», «si el código contradice una memoria, gana el código y la memoria se marca `stale`», «la rama como cuarentena») y un informe de estado del arte que la pone como el diferenciador que «casi ningún competidor» resuelve («obsolescencia ligada a git», P1.6 de su hoja de ruta). Esta exploración contrasta esas ideas con `develop` en 0.68.0 (`4604b35`). Lo que viene de la documentación de Claude Code dice qué página se leyó.

## Premisas de la investigación, verificadas contra 0.68.0

| Premisa | Resultado |
|---|---|
| clawg no detecta memorias obsoletas por cambios de código | **Cierta.** Ninguna tabla guarda una ruta, un hash o un commit por memoria: `memory` (`crates/clawg-core/migrations/V1__init.sql:253`) y las migraciones hasta V31 no tienen columnas de git, y `SaveRequest` sólo lleva texto, agente, proyecto, tipo, importancia, `derived_from`, `user_stated`, `operation_id` y `topic_key` (`crates/clawg-cli/src/daemon/api.rs:387-411`). Las coincidencias de `stale` en `crates/clawg-core/src/` son de locks, sockets y huérfanas, no de memorias |
| Hace falta un índice de símbolos con hash para anclar | **Falsa en lo que pide.** Ya existe un índice por archivo con hash: `project_doc_file` guarda `path` y `content_hash` de cada archivo indexado del proyecto (`crates/clawg-core/migrations/V12__project_docs.sql:10-17`), y el refresco lo compara con el disco cada vez que corre (`plan_refresh`, `crates/clawg-core/src/memory/project_docs/mod.rs:188`). Anclar por archivo alcanza para el aviso; por símbolo es un refinamiento que se decide con datos (paso 3) |
| La memoria y el índice deberían ser direccionados por hash para que los worktrees compartan | **Cierta en lo que comparten y con un hallazgo en contra.** Un worktree resuelve a la raíz del repositorio principal (`root_from_marker`, `crates/clawg-core/src/memory/identity.rs:144-159`), así que comparte proyecto, memorias e índice de docs. Pero el índice lee los archivos de esa raíz (`root_of`, `project_docs/mod.rs:679-695`): una sesión en un worktree con otra rama recibe de `search_project` los pasajes del checkout principal, no los de su rama. Para anclas eso importa: el hash de referencia tiene que salir del checkout donde se guardó, no del índice (decisión D2) |
| «Si el código contradice una memoria, gana el código» | **Ya está, como regla para el agente:** «check what it claims against the code or ask the user» (`protocol.md:155`), con el resto de [untrusted-recall](../archive/2026-10-20-untrusted-recall/proposal.md) (52). Lo que falta es la señal que le diga cuándo mirar |
| El linaje `derived_from` degrada en cascada lo que se derivó de una memoria equivocada | **Ya existe la parte que sirve.** `synthesize` lista las conclusiones que perdieron una fuente (borrada, reemplazada o purgada) en `to_review` (`openspec/specs/synthesis/spec.md:151-156`), y `session-start` lo avisa ([conclusion-upkeep](../archive/2026-10-19-conclusion-upkeep/proposal.md), 45). Un ancla cambiada podría sumarse como otra forma de «fuente que ya no vale» (paso 3), sin cascada automática |
| La rama como cuarentena: lo de una rama vale sólo en esa rama hasta el merge | **Sin base hoy, y cara.** Un proyecto es el remoto (`origin_key`, `.claude/CLAUDE.md`, perfil de datos) y no sabe de ramas. Una cuarentena por rama pide leer el estado de git en cada lectura (qué ramas existen, cuál se mergeó, qué commit se revirtió) y cambia qué ve cada sesión. Este cambio **sólo registra** la rama y el commit del save (paso 1), que es lo que haría falta para decidirlo después |

## Qué problema resuelve y cuánto pesa

El problema existe: un `semantic_fact` del estilo «`plan_refresh` compara `model_id`» o una causa raíz «el bug estaba en `worker.rs` porque…» sigue saliendo con el mismo puntaje después de que el archivo se reescribió. Los tiers 1 nunca decaen (una memoria con importancia 9 o más «is never demoted for not being used», `protocol.md:111`), así que una decisión vieja anclada a código que cambió es la que más tiempo dura.

Lo que **no** se sabe es cuánto pesa en un store real: cuántas memorias vivas nombran un archivo del proyecto, y de ésas cuántas apuntan a un archivo que cambió desde que se guardaron. Por eso el paso 0 mide antes de escribir código.

## Enfoques

| Enfoque | A favor | En contra |
|---|---|---|
| **A. Ancla por archivo, con hash al guardar (recomendado)** | Usa lo que ya hay: rutas en el texto y archivos en el índice o en el disco. Determinista y sin LLM. Un archivo cambiado es una señal barata de comprobar en cada lectura | Un archivo cambia por cualquier motivo: un aviso por línea tocada es ruido. Hace falta medir la tasa de avisos falsos (paso 2) |
| B. Ancla por símbolo (tree-sitter), con el hash del cuerpo del símbolo | Más fina: sólo avisa si cambió la función nombrada. `code.rs` ya parte Rust, Python y TypeScript por el árbol sintáctico ([project-docs](../../specs/project-docs/spec.md)) | Pide un índice de símbolos que hoy no existe (las piezas de código son pasajes, no símbolos con id estable). Se decide con los avisos falsos de A |
| C. Comparar con `git log` desde el commit del save | No necesita el índice; da «N commits tocaron este archivo desde el save» | Corre `git` en cada lectura o en un trabajo de fondo; el daemon hoy sólo lee la raíz y el primer commit de git, con plazo de 1 s (`.claude/CLAUDE.md`, perfil de datos). Un repositorio sin git queda fuera |
| D. Que el agente marque `stale` con una herramienta de feedback | Simple de implementar | Depende de que el modelo se acuerde, que es justo lo que la investigación desaconseja; suma una herramienta al contrato. Se pospone y se mide en 96 |

## Pasos propuestos

**Paso 0 — medir sin código (S).** Sobre un export (`clawg export --include-deleted`) y el árbol del proyecto: cuántas filas vivas nombran una ruta que existe en el proyecto, por tipo y tier; de ésas, cuántas apuntan a un archivo cuyo último commit es posterior al `created_at` de la fila (con `git log -1 --format=%ct -- RUTA`). Se corre sobre este repositorio y, con autorización, sobre el perfil dev. Si menos de un 5 % de las filas vivas nombran un archivo, el cambio se cierra sin código y queda en «Ideas evaluadas».

**Paso 1 — registrar (M).** En la transacción que termina el save, dos tablas nuevas en V32: `memory_origin` (una fila por save: la cima del checkout donde se guardó, y `head_commit` y `branch` cuando hay git), que también usa la parte A de [session-lifecycle](../session-lifecycle/exploration.md) (99), y `memory_anchor` (`memory_id`, `path` relativo a esa cima y `content_hash` del archivo en ese momento). Las rutas salen del texto enmascarado con una función pura nueva (tokens con `/` o `.` que **existen** como archivo en el checkout del que guarda: la existencia es el filtro, no una regex), que el puente o el hook ya conocen por su `path` registrado. Una fila sin rutas no tiene ancla. Las filas anteriores no se tocan: sin ancla no hay aviso.

**Paso 2 — avisar (M).** `search_memory`, `read_memory` y los bloques de `recall_context` agregan `changed=RUTA` a la línea de un hit cuyo archivo ya no tiene el hash del ancla (o `gone=RUTA` si ya no existe), como un marcador más de los que fija `agent_contract.json`. Ningún puntaje cambia. Se mide con `rag-eval --agent-view` (tokens) y se cuentan los avisos falsos sobre el store del paso 0: un aviso es falso cuando la memoria sigue siendo cierta.

**Paso 3 — decidir con datos (S cada uno).** Según la tasa de avisos falsos: anclar por símbolo (B), bajar el puntaje de lo cambiado (un cambio de ranking que pasa por `rag-eval --diff` con umbral fijado antes), sumar las memorias con ancla cambiada a la revisión de `synthesize`, o una cuarentena por rama sobre `branch`.

## Decisiones abiertas

- **D1. Qué cuenta como ruta.** Sólo archivos del checkout (recomendado) o también directorios y rutas con línea (`archivo.rs:120`). Un directorio cambia demasiado seguido para ser útil.
- **D2. De dónde sale el hash de referencia.** Del disco del checkout donde se guardó (recomendado: un worktree tiene sus propios archivos) o de `project_doc_file` (más barato, pero es el del checkout principal y sólo existe con `index_docs`).
- **D3. Cuándo se compara.** En cada lectura, con el hash calculado en el hilo bloqueante y una caché por `mtime`, o en un trabajo de fondo junto al refresco del índice (`REFRESH_INTERVAL` 60 s, `project_docs/mod.rs:32`). Lo decide la latencia de `/v1/search`, que ya mide `http.duration` por ruta ([`specs/metrics`](../../specs/metrics/spec.md)).
- **D4. Qué hace `import`.** El formato `clawg-memories` no tiene anclas; un import las deja vacías (una máquina distinta no tiene el mismo checkout) o las trae como dato del origen sin compararlas. Recomendado: no las exporta en la versión 1.

## Riesgos

- **Ruido.** Si casi todo aviso es falso, el agente aprende a ignorarlo, que es peor que no tenerlo. Por eso el paso 2 no toca puntajes y el paso 3 sólo actúa con la tasa medida.
- **Datos.** V32 es una tabla nueva, sin tocar `memory`; un store anterior abre igual y una versión anterior de clawg la ignora, pero la regla de `config.yaml` pide el test de preservación con datos sembrados en V31.
- **Privacidad.** Una ruta y un hash no son texto de la memoria, pero sí dicen qué archivos tocó el usuario; quedan en `memory.db` como el resto, nunca en `metrics.db`.

## Esfuerzo e impacto

| Paso | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|
| 0 | Decide si el resto vale | Bajo | Bajo (horas) |
| 1 | Ninguno visible; habilita 2, 3 y la parte de worktree de 99 | Medio (migración) | Medio |
| 2 | Alto si la tasa de avisos falsos es baja | Medio | Medio |
| 3 | Según los datos | Según el paso | Bajo a Medio cada uno |

## Fuentes

- Código y specs de `develop` en `4604b35` (0.68.0), con las rutas citadas.
- https://code.claude.com/docs/en/tools-reference (sección «LSP tool behavior», leída el 2026-10-31): Claude Code ya ofrece definiciones y referencias por un servidor de lenguaje cuando se instala un plugin de code intelligence, y no en las sesiones en la nube. Por eso este cambio no propone un índice de símbolos y referencias propio; ver «Ideas evaluadas» en el [README](../../README.md).
- Los dos documentos de investigación previos, para la motivación y la comparación con otras herramientas, que no se volvieron a verificar.
