# memory-anchors — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 (y 3) | M–L, por pasos (el paso 0 es S, sólo lee y no cambia código) | — para los pasos 0 a 2; el índice de docs de [project-indexing](../archive/2026-10-15-project-indexing/proposal.md) (15, archivado) sólo si D5 elige la tabla de nombres. Se coordina con [session-lifecycle](../session-lifecycle/exploration.md) (99), que lee el mismo contexto git del save; con [agent-eval](../agent-eval/exploration.md) (96), que mide si el aviso cambia lo que hace el agente; y con [memory-module-layout](../archive/2026-11-13-memory-module-layout/exploration.md) (105), que mueve casi todos los archivos que las memorias nombran | exploración (borrador) |

**Tesis:** una memoria que habla de código envejece cuando el código cambia, y hoy clawg no lo puede saber. El protocolo pide escribir cada save «con las rutas y los identificadores exactos» (`crates/clawg-cli/src/mcp/protocol.md:14-16`) y dice que, si una memoria contradice al código, se compruebe contra el código (`protocol.md:148-155`), pero nada le avisa al agente **cuál** memoria nombra algo que ya no está. La propuesta es anclar cada save a los archivos y los nombres del proyecto que cita y que existían al guardarlo (más el commit y la rama de ese momento) y, al leerlo, marcar `gone=` cuando uno de ellos ya no existe. Que un archivo citado haya cambiado no se marca: en este repositorio, a las dos semanas cambió el 72 % de los archivos que cita un documento escrito como un save, así que esa marca saldría en casi todas las memorias de código y el agente aprendería a ignorarla (ver «Cuánto avisa cada señal»). La marca es evidencia para el agente: no oculta, no borra y, al principio, no cambia ningún puntaje.

**De dónde sale.** De tres documentos de investigación previos, que no están en el repositorio: un diseño de integración con Claude Code («memoria anclada al índice», «si el código contradice una memoria, gana el código y la memoria se marca `stale`», «la rama como cuarentena»), un informe de estado del arte que la pone como un problema que casi nadie resuelve («obsolescencia ligada a git», P1.6 de su hoja de ruta) y un estudio sobre 0.36.0 que prototipó la extracción de referencias, la etiquetó a mano y midió cuántos nombres citados dejan de existir. Esta exploración contrasta esas ideas con `develop` en 0.68.0 (`4604b35`); la sección «Cuánto avisa cada señal» y los datos de D1 y D5 se midieron en 0.80.0 (`acbf515`). Lo que viene de la documentación de Claude Code dice qué página se leyó.

## Premisas de la investigación, verificadas contra 0.68.0

| Premisa | Resultado |
|---|---|
| clawg no detecta memorias obsoletas por cambios de código | **Cierta.** Ninguna tabla guarda una ruta, un hash o un commit por memoria: `memory` (`crates/clawg-core/migrations/V1__init.sql:253`) y las migraciones hasta V32 no tienen columnas de git, y `SaveRequest` sólo lleva texto, agente, proyecto, tipo, importancia, `derived_from`, `user_stated`, `operation_id` y `topic_key` (`crates/clawg-cli/src/daemon/api.rs:387-411`). Las coincidencias de `stale` en `crates/clawg-core/src/` son de locks, sockets y huérfanas, no de memorias |
| Hace falta un índice de símbolos con hash para anclar | **Falsa en lo que pide, cierta en lo que busca.** Para guardar un ancla por archivo alcanza con lo que hay: `project_doc_file` guarda `path` y `content_hash` de cada archivo indexado (`crates/clawg-core/migrations/V12__project_docs.sql:10-17`), y el disco del checkout también los da. Pero el hash de un archivo no sirve como aviso («Cuánto avisa cada señal»): lo que avisa es que un nombre citado dejó de existir. Para eso no hace falta un hash por símbolo, sino saber si el nombre sigue ahí (D5) |
| La memoria y el índice deberían ser direccionados por hash para que los worktrees compartan | **Cierta en lo que comparten y con un hallazgo en contra.** Un worktree resuelve a la raíz del repositorio principal (`root_from_marker`, `crates/clawg-core/src/memory/identity.rs:144-159`), así que comparte proyecto, memorias e índice de docs. Pero el índice lee los archivos de esa raíz (`root_of`, `project_docs/mod.rs:679-695`): una sesión en un worktree con otra rama recibe de `search_project` los pasajes del checkout principal, no los de su rama. Para anclas eso importa: el hash de referencia tiene que salir del checkout donde se guardó, no del índice (decisión D2) |
| «Si el código contradice una memoria, gana el código» | **Ya está, como regla para el agente:** «check what it claims against the code or ask the user» (`protocol.md:155`), con el resto de [untrusted-recall](../archive/2026-10-20-untrusted-recall/proposal.md) (52). Lo que falta es la señal que le diga cuándo mirar |
| El linaje `derived_from` degrada en cascada lo que se derivó de una memoria equivocada | **Ya existe la parte que sirve.** `synthesize` lista las conclusiones que perdieron una fuente (borrada, reemplazada o purgada) en `to_review` (`openspec/specs/synthesis/spec.md:151-156`), y `session-start` lo avisa ([conclusion-upkeep](../archive/2026-10-19-conclusion-upkeep/proposal.md), 45). Una memoria con un `gone` podría sumarse como otra forma de «fuente que ya no vale» (D6, paso 3), sin cascada automática |
| La rama como cuarentena: lo de una rama vale sólo en esa rama hasta el merge | **Sin base hoy, y cara.** Un proyecto es el remoto (`origin_key`, `.claude/CLAUDE.md`, perfil de datos) y no sabe de ramas. Una cuarentena por rama pide leer el estado de git en cada lectura (qué ramas existen, cuál se mergeó, qué commit se revirtió) y cambia qué ve cada sesión. Este cambio **sólo registra** la rama y el commit del save (paso 1), que es lo que haría falta para decidirlo después, y los usa para no marcar desde otra rama (paso 2) |

## Qué problema resuelve y cuánto pesa

El problema existe: un `semantic_fact` del estilo «`plan_refresh` compara `model_id`» o una causa raíz «el bug estaba en `worker.rs` porque…» sigue saliendo con el mismo puntaje después de que la función se renombró o el archivo se borró. Los tiers 1 nunca decaen (una memoria con importancia 9 o más «is never demoted for not being used», `protocol.md:111`), así que una decisión vieja anclada a código que cambió es la que más tiempo dura.

Lo que **no** se sabe es cuánto pesa en un store real: cuántas memorias vivas nombran un archivo o un identificador del proyecto, y de ésas cuántas citan algo que ya no existe. Por eso el paso 0 mide antes de escribir código.

## Cuánto avisa cada señal

Medido en 0.80.0 (`acbf515`) sin memorias reales, sobre el corpus más parecido que tiene el repositorio: los documentos de los 102 cambios archivados de openspec, escritos como un save del protocolo (un tema; qué, por qué y dónde; rutas e identificadores entre comillas invertidas) por sesiones de agentes sobre este mismo código. Para cada carpeta se tomaron las rutas citadas que existían en el último commit de la fecha de la carpeta, y se compararon con HEAD con `git diff --no-renames`:

| Edad del documento | Rutas que resolvían | Iguales | Cambiadas | Ya no existen |
|---|---|---|---|---|
| hasta 7 días | 322 | 54 % | 46 % | 0 |
| 8 a 14 días | 449 | 28 % | 72 % | 0 |
| 15 a 21 días | 1.498 | 17 % | 82 % | 3 |
| 22 días o más | 847 | 6 % | 93 % | 5 |

Lo que dice:

- **Un archivo cambiado no es un aviso.** A la semana cambió casi la mitad de lo citado, y a las tres semanas, cuatro de cada cinco. De los 221 archivos `.rs` de `crates/`, 110 cambiaron en los últimos 7 días y 163 en los últimos 14. Una marca `changed=` saldría en la mayoría de los hits que citan código, sin decir si lo que la memoria afirma sigue valiendo.
- **Un archivo que desaparece casi no pasa:** 8 de 3.116 (0,3 %). Una marca sólo por ruta borrada sería precisa pero casi nunca aparecería, salvo en un refactor que mueva módulos, que la dispararía en masa (D1, Riesgos).
- **Lo que se pierde de verdad es el nombre.** El estudio sobre 0.36.0 encontró que, en los documentos archivados de 1 a 8 días, el 6 % de los símbolos citados (746 de 12.487) ya no estaba definido. En una muestra de 45, la mitad era código borrado o renombrado; el resto eran nombres de dependencias, del sistema operativo o cosas que no eran código. Ésos nunca resuelven al guardar, así que anclar sólo lo que resolvía los deja fuera. Su extractor, anclado así, acertó 47 de 48 referencias y encontró 47 de 53, con un solo anotador.
- **Límites:** son documentos, no saves. Citan más código por párrafo, y justo el código que se estaba cambiando. La proporción de memorias reales que citan código, y cuánto de eso se pierde, la mide el paso 0.

## Enfoques

| Enfoque | A favor | En contra |
|---|---|---|
| A. Ancla por archivo, con aviso cuando cambia su hash | Usa lo que ya hay; determinista y sin LLM | **Descartado como aviso por la medición:** marcaría casi todas las memorias de código a las dos semanas. El hash se guarda igual, como dato (paso 1) |
| **B. Ancla por archivo y por nombre, con aviso sólo cuando algo ya no existe (recomendado)** | Avisa lo que la medición muestra que se pierde: una función o un archivo que se borró o se renombró. Es raro y preciso, así que el agente no aprende a ignorarlo | Pide saber si un nombre sigue existiendo (D5). Un archivo movido sin cambios se lee como borrado si no se lo distingue (D1) |
| C. Comparar con `git log` desde el commit del save | No necesita el índice; da «N commits tocaron este archivo desde el save» | Es la señal de A, con el mismo ruido. Corre `git` en cada lectura o en un trabajo de fondo; el daemon hoy sólo lee la raíz y el primer commit de git, con plazo de 1 s (`.claude/CLAUDE.md`, perfil de datos). Un repositorio sin git queda fuera |
| D. Que el agente marque `stale` con una herramienta de feedback | Simple de implementar | Depende de que el modelo se acuerde, que es justo lo que la investigación desaconseja; suma una herramienta al contrato. Se pospone y se mide en 96 |

## Pasos propuestos

**Paso 0 — medir sin código (S).** La parte de archivos ya está medida sobre openspec. Falta medirla sobre memorias reales y sumar los nombres. Sobre un export (`clawg export --include-deleted`) y el historial del proyecto, con un script fuera del producto:

1. cuántas filas vivas, por tipo y tier, nombran una ruta o un identificador que existía en el commit de su `created_at`;
2. de ésas, cuántas citan hoy algo que ya no existe: la ruta, o el identificador en el archivo citado donde aparecía (B1 de D5);
3. en una muestra de 30 de las que darían `gone=`, cuántas están de verdad viejas (lo que afirman ya no vale), etiquetadas antes de ver el resultado del script.

Se corre sobre este repositorio y, con autorización, sobre el perfil dev. El cambio se cierra sin código y queda en «Ideas evaluadas» si menos de un 5 % de las filas vivas nombran código del proyecto, o si menos de 21 de las 30 (70 %) están de verdad viejas: un aviso que se equivoca más que eso enseña a ignorarlo.

**Paso 1 — registrar (M).** En la transacción que termina el save, dos tablas nuevas en la próxima migración libre (hoy V33): `memory_origin` (una fila por save: la cima del checkout donde se guardó, y `head_commit` y `branch` cuando hay git), que también usa la parte A de [session-lifecycle](../session-lifecycle/exploration.md) (99), y `memory_anchor` (una fila por ruta citada, con su `content_hash` de ese momento, y una por nombre citado, con la ruta donde se encontró; D5).

- **Qué se ancla:** sólo lo que resuelve en el checkout de quien guarda en ese momento; la existencia es el filtro, no una regex. Un nombre de una dependencia, una variable de entorno o un nombre inventado nunca se ancla, así que tampoco puede «desaparecer». Las rutas y los nombres salen del texto enmascarado con una función pura nueva, con tabla de casos. `identifier_tokens` sola no alcanza: `fts_tokens` corta `Store::insert` y `cli.rs` en palabras que `is_identifier` rechaza (`crates/clawg-core/src/memory/search.rs:660-664`, `:830-835`), así que la función suma las rutas, los nombres `A::B` y un largo mínimo que descarta `v1` o `e5`.
- **Un save largo** se parte en piezas (`memory_piece`); las anclas se extraen del texto entero y cuelgan del id de la primera pieza, como hace `memory_piece`.
- **Sin anclas:** una memoria sin proyecto o con `all_projects` (no hay un checkout contra el cual validarla) y las filas anteriores a la migración. Sin ancla no hay aviso.
- **La rama y el commit** los lee el puente, que conoce su cwd y por eso su worktree; el daemon sólo conoce la raíz registrada, que para un worktree es la del principal. Se leen de `.git` sin lanzar `git`, sobre el lector de `identity.rs` (marcador `.git`, `gitdir` y `commondir`): el `HEAD` propio del worktree; si dice `ref: refs/heads/<rama>`, la ref suelta en el directorio común y, si no está, `packed-refs` (líneas `<id> <ref>`, una cabecera `#` y líneas `^` de refs peladas); un id suelto es un HEAD separado, sin rama. Los ids pueden tener 40 o 64 hexadecimales (SHA-256). Un repositorio reftable deja en `HEAD` el talón `ref: refs/heads/.invalid`: rama y commit quedan vacíos, sin leer el formato binario, y ese caso tiene test desde el primero, porque Git 3.0 planea hacer de reftable el formato por defecto. Nunca se guarda el remoto ni una ruta absoluta.

**Paso 2 — avisar (M).** `search_memory`, `read_memory` y los bloques de `recall_context` agregan `gone=RUTA` o `gone=NOMBRE` a la línea de un hit cuando algo de lo que anclaba ya no existe, como un marcador más de los que fija `agent_contract.json`, con una oración en `protocol.md` que diga qué hacer: comprobarlo en el código y guardar la versión corregida bajo su `topic_key`, o dejarla si lo que explica es por qué se borró algo.

- **Sólo desde la misma rama.** Se marca cuando el checkout que lee está en la misma rama que el save (`memory_origin.branch`); en otra rama, en un HEAD separado o sin git, no se marca nada. Sin esa regla, una memoria guardada en una rama de trabajo que nombra un archivo nuevo saldría `gone` en `main`, y lo mismo entre dos clones que comparten proyecto por el remoto.
- **Un archivo cambiado no va en los hits.** En `read_memory`, que el agente pide de a una, puede ir la línea de origen (rama y commit) y qué archivos anclados cambiaron: ahí cuesta una línea y el agente ya decidió mirar esa memoria.
- Ningún puntaje cambia. Se mide con `rag-eval --agent-view` (tokens) y con los avisos falsos sobre el store del paso 0.

**Paso 3 — decidir con datos (S cada uno).** Según la tasa de avisos falsos y lo que se pierde: la tabla de nombres definidos (B2 de D5) si B1 deja fuera muchos nombres que se citan sin su archivo; sumar las memorias con un `gone` a la revisión de `synthesize` (D6); bajar el puntaje de lo marcado (un cambio de ranking que pasa por `rag-eval --diff` con umbral fijado antes); o una cuarentena por rama sobre `branch`.

## Decisiones abiertas

- **D1. Qué cuenta como ruta.** Sólo archivos del checkout (recomendado) o también directorios y rutas con línea (`archivo.rs:120`). Un directorio cambia demasiado seguido para ser útil, y una línea corrida es la señal más ruidosa: se guarda la ruta sin la línea.
  - **Rutas parciales y nombres sueltos.** En el corpus de openspec, 453 citas eran una ruta parcial de un archivo real (`daemon/api.rs`, `rag-eval/main.rs`) y 562, un nombre de archivo suelto que el árbol tiene más de una vez (hay 13 `mod.rs` y 2 `api.rs`). Se propone resolver una y otra por sufijo, sólo cuando el sufijo es único en el checkout. Lo que no resuelve (fuentes de dependencias, archivos del directorio de datos como `daemon.json`, el set dorado gitignoreado) no se ancla.
  - **Archivos movidos.** Si una ruta anclada ya no existe pero su nombre de archivo aparece una sola vez en otra carpeta del checkout, se lee como movido y se reancla sin aviso; con B2, también si sus nombres anclados siguen definidos en otro archivo. Sin una regla así, un refactor como 105 haría saltar el aviso en casi toda memoria de `clawg-core`.
- **D2. De dónde sale el hash de referencia, y quién resuelve.** Del disco del checkout donde se guardó (recomendado: un worktree tiene sus propios archivos) o de `project_doc_file`, más barato, pero es el del checkout principal y sólo existe con `index_docs`. Con el disco, conviene que resuelva el puente, que manda las anclas con el save: viajan en la fila del outbox, así que una reproducción no depende del árbol del momento en que se reproduce.
- **D3. Cuándo se compara.** En cada lectura, con el hash calculado en el hilo bloqueante y una caché por `mtime`, o en un trabajo de fondo junto al refresco del índice (`REFRESH_INTERVAL` 60 s, `project_docs/mod.rs:32`). Lo decide la latencia de `/v1/search`, que ya mide `http.duration` por ruta ([`specs/metrics`](../../specs/metrics/spec.md)).
- **D4. Qué hace `import`.** El formato `clawg-memories` no tiene anclas; un import las deja vacías (una máquina distinta no tiene el mismo checkout) o las trae como dato del origen sin compararlas. Recomendado: no las exporta en la versión 1.
- **D5. Cómo se sabe que un nombre sigue existiendo.**
  - **B1, recomendado para empezar:** el nombre se ancla a la ruta citada en la que aparece al guardar, y está `gone` cuando ese archivo ya no lo contiene como palabra. No pide un índice nuevo, sirve para cualquier archivo de texto (SQL, YAML, Markdown) y el paso 0 lo puede medir con el historial. No ancla un nombre que la memoria cita sin su archivo, y un nombre que se mudó de archivo se lee como borrado de ése, lo que es cierto de la memoria, que dice dónde estaba.
  - **B2:** una tabla de nombres definidos por proyecto (`project_doc_symbol(path, name, kind)`), llenada en el mismo refresco del índice de docs desde el árbol que `code.rs` ya parsea, con la consulta `TAGS_QUERY` que exportan las tres gramáticas de `Cargo.lock` más consultas propias: la de `tree-sitter-rust` 0.24.2 no captura `const` ni `static`, ni variantes ni campos, y la de `tree-sitter-typescript` 0.23.2 sólo trae firmas, interfaces, módulos y clases abstractas. Ancla los nombres sin ruta y ve viva una función que se movió. Sólo cubre Rust, Python y TypeScript dentro del tope del índice, pide una migración y un reindexado, y si la búsqueda la lee hay que cambiar «Indexar no cambia las memorias» de [`specs/project-docs`](../../specs/project-docs/spec.md). No es el índice estructural que «Ideas evaluadas» descartó: aquél era para navegar (definiciones, referencias, tests) y lo cubre el LSP del cliente; éste es un conjunto de nombres que consulta el daemon, que no tiene ese LSP a su alcance, como tampoco lo tienen las sesiones en la nube.
- **D6. Dónde aparece el aviso primero.** En la línea del hit (recomendado: el agente lo ve en el momento en que decide si confiar en la memoria, y con sólo `gone` se espera en pocos hits) o en una cola de `synthesize` al estilo de `to_review`, con una línea en `recall_context` y `session-start`. La cola no toca el contrato del agente ni el texto de los hits, pero llega una vez por sesión, lejos del momento de usar la memoria, y una cola larga se deja de leer.

## Riesgos

- **Ruido.** Si casi todo aviso es falso, el agente aprende a ignorarlo, que es peor que no tenerlo. La medición ya descartó el aviso por archivo cambiado; el paso 0 mide el de `gone` antes de escribir código, y el paso 2 no toca puntajes.
- **Refactors que mueven archivos.** 105 propone repartir los 51 módulos de `crates/clawg-core/src/memory/` en carpetas por dominio. Sin la regla de movidos de D1, cada memoria anclada a uno pasaría a `gone` aunque siga valiendo. Si 105 va antes que el paso 1, las anclas nacen sobre la estructura final.
- **Datos.** La migración son dos tablas nuevas, sin tocar `memory`; un store anterior abre igual y una versión anterior de clawg las ignora, pero la regla de `config.yaml` pide el test de preservación con datos sembrados en la versión anterior.
- **Privacidad.** Una ruta, un nombre y un hash no son texto de la memoria, pero sí dicen qué archivos tocó el usuario; quedan en `memory.db` como el resto, nunca en `metrics.db`.

## Esfuerzo e impacto

| Paso | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|
| 0 | Decide si el resto vale; la parte de archivos ya está medida | Bajo | Bajo (horas) |
| 1 | Ninguno visible; habilita 2, 3 y la parte de worktree de 99 | Medio (migración) | Medio |
| 2 | Alto si la tasa de avisos falsos es baja | Medio | Medio |
| 3 | Según los datos | Según el paso | Bajo a Medio cada uno |

## Qué no se toma del estudio sobre 0.36.0

- **Saber si el commit del save es antepasado de HEAD.** Pide leer la base de objetos de git (packfiles o `commit-graph`) o una dependencia como `gix` para leer tres archivos, falla en un clone superficial, y lo que respondería (si la rama del save ya se mergeó) lo cubre la regla de la rama del paso 2.
- **Un subconjunto de `rag-eval` con un árbol de fixture y memorias viejas a propósito.** El paso 2 no cambia el ranking, así que `rag-eval` sólo tiene que medir los tokens, y lo que el aviso cambia en el agente lo mide 96. Se suma si el paso 3 prueba bajar el puntaje.

## Fuentes

- Código y specs de `develop` en `4604b35` (0.68.0), con las rutas citadas. Las líneas de `search.rs`, las versiones de las gramáticas y sus `tags.scm` (leídos en el registro de cargo) son de `acbf515` (0.80.0).
- La medición de «Cuánto avisa cada señal» y de D1: HEAD `acbf515`; los `*.md` de las 102 carpetas de `openspec/changes/archive/`; las rutas entre comillas invertidas, sin el sufijo `:línea`; el commit de cada carpeta con `git rev-list -1 --before=<fecha>`; y `git ls-tree` y `git diff --no-renames --name-only` contra HEAD.
- https://code.claude.com/docs/en/tools-reference (sección «LSP tool behavior», leída el 2026-10-31): Claude Code ya ofrece definiciones y referencias por un servidor de lenguaje cuando se instala un plugin de code intelligence, y no en las sesiones en la nube. Por eso este cambio no propone un índice de símbolos y referencias para navegar; ver «Ideas evaluadas» en el [README](../../README.md) y D5.
- El formato de `.git` del paso 1 (`HEAD` por worktree, `packed-refs`, el talón de reftable y los planes de Git 3.0) sale del estudio sobre 0.36.0, que lo comprobó con git 2.43.0 y citó gitrepository-layout, git-pack-refs, git-worktree y BreakingChanges (git-scm.com/docs) y la guía de reftable de GitLab (2024-05-30). No se volvió a leer.
- Los tres documentos de investigación previos, para la motivación; sólo las cifras del estudio sobre 0.36.0 se usan como evidencia, con sus límites.
