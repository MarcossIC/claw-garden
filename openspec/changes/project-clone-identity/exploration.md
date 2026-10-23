# project-clone-identity — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 | M | project-root-guard (define qué es una colisión y cómo se avisa; este cambio la redefine) | exploración (borrador) |

**Tesis:** la identidad de un proyecto es la ruta del disco, no el repositorio. Dos clones del mismo repo son dos proyectos y sus memorias se parten. Quien tiene una copia de trabajo y otra de revisión, o vuelve a clonar un repo en otra carpeta, pierde la memoria que juntó. El caso de los worktrees, que se planteaba como el principal, **ya está resuelto**; lo que queda es el clone y los bordes del worktree.

## Premisas verificadas

| Premisa | Veredicto |
|---|---|
| La identidad es la raíz git canónica, hasheada | **Cierto** (`crates/clawg-core/src/memory/identity.rs:41-53`, `:11-15`). |
| Dos worktrees del mismo repo son proyectos distintos | **Falso para el caso común.** Un worktree vinculado tiene un archivo `.git` con `gitdir:`; el resolutor lee `<gitdir>/commondir` y devuelve la carpeta del repo principal (`identity.rs:55-68`, `:87-96`). Ya hay test con un worktree armado a mano (`crates/clawg-core/tests/project_root.rs:38-50`) y spec (`specs/project` R1). El comando `git rev-parse --git-common-dir` en este repo devuelve `.git`, que es el mismo dato que `commondir` expresa. |
| Dos clones del mismo repo son proyectos distintos | **Cierto.** El resolutor no mira remotos ni `.git/config`: `identity.rs` entero no contiene la palabra `origin` ni lee configuración. El código de producción no llama a `git` ni usa `git2`/`gix` (sólo `rag-eval` ejecuta `git`, `crates/clawg-eval/src/bin/rag-eval/report.rs:565`). |
| Los worktrees de un repo bare o con `--separate-git-dir` | **Hueco real, chico.** Si el directorio común no se llama `.git`, el worktree queda como su propia raíz (`identity.rs:92-95`; test `a_worktree_of_a_bare_repository_is_its_own_root`, `identity.rs:261-267`). Es una decisión explícita, porque no hay árbol de trabajo principal al que apuntar. |
| Unir dos ids con `register_project` es mover filas y nada más | **Falso.** El guard de `project.rs:105-115` salta los ids legados que pertenecen a otra raíz registrada; para el resto inserta en `project_rekey` y corre `UPDATE {table} SET project_id = ?2 WHERE project_id = ?1` (`project.rs:116-129`). No reconcilia `content_hash` repetidos ni `topic_key` vivos repetidos en el scope (agente, proyecto) que resulta, y no llama a `revise_in` (`topic.rs:111`). El re-key es un escritor de `memory` fuera del worker de ingesta, cuya serialización es lo que mantiene esas dos invariantes. |
| Un submódulo | Es su propio proyecto, a propósito (`identity.rs:70-79`; `project_root.rs:53-63`). No se toca. |

## Problema (verificado contra el código)

### Qué parte la memoria hoy

- El id es `{nombre de carpeta}-{16 bits de SipHash de la ruta}` (`identity.rs:11-15`, `:146-153`). Dos clones en `C:\work\app` y `D:\review\app` dan dos ids, sin relación.
- Una sesión de un clone nunca ve lo que guardó la sesión del otro: toda búsqueda filtra por `m.project_id = ?` (`crates/clawg-core/src/memory/search.rs:207`, `:268`). La excepción son `user_preference` y las filas sin proyecto (`search.rs:207`).
- Mover un repo de carpeta (renombrar, cambiar de disco) cambia la ruta y por lo tanto el id. Las filas quedan bajo el id viejo, que ninguna sesión nueva calcula. `project_rekey` sólo recibe los `legacy_ids` que el puente conoce y el puente sólo envía uno: el de la ruta cruda (`crates/clawg-cli/src/mcp/server.rs:870-871`). Un repo movido no tiene forma de reclamar su pasado sin SQL.
- Lo que sí está bien: `register_project` ya sabe mover filas de varios ids a uno, deja registro por fila y es idempotente (`crates/clawg-core/src/memory/project.rs:56-124`). La maquinaria existe; lo que falta es **qué id es el correcto** y **cómo se enteran los ids viejos**.

### Dónde vive un `project_id` (todo lo que habría que mover)

| Dónde | Qué guarda | Lo cubre `project_rekey` hoy |
|---|---|---|
| `memory`, `episodic_trajectory`, `procedural_skill` | `project_id` (`crates/clawg-core/migrations/V1__init.sql:9`, `:66`, `:117`) | **Sí** (`project.rs:17`) |
| `memory_fts` | `project_id UNINDEXED`, contenido externo: lee de `memory` (`V8__memory_fts.sql:39`; `project.rs:6-7`) | No hace falta |
| `project` | `id`, `root_path` (`V9__project.sql:6-10`) | No: es la tabla de raíces |
| `project_rekey` | `legacy_id`, `project_id` (`V9__project.sql:17-24`) | Es el registro mismo |
| `project_doc_file`, `project_doc_chunk` | `project_id` (`V12__project_docs.sql:10-31`) | **No** |
| `project_doc_vectors` | `project_id` como clave de partición (`V12:36-39`) | **No** |
| `project_doc_fts` | `project_id UNINDEXED` (`V12:45-49`) | **No** |
| `synthesis_review` | `scope_key` = id de proyecto o `''` (`V10__synthesis.sql:22-27`) | **No** |
| Disco: `<workspaces>/<agente>/<id>/workspace.md` | el id en la ruta (`workspace.rs:43-46`) | Sólo copia (`adopt_legacy`, `workspace.rs:56-71`; `project.rs:117-121`); deja el original |
| `sharing.json` | agentes, no proyectos (`sharing.rs` no menciona proyectos) | No aplica |
| Vistas y `memory_vectors` | las vistas leen `project_id` de las tablas (`V3__view.sql:7-17`); los vectores no lo llevan | No hace falta |

Dos hallazgos que condicionan el diseño:

1. **El índice de documentos y `synthesis_review` no siguen un re-key.** Hoy no hace falta (el id nuevo se registra y se reindexa desde cero), pero con ids que cambian dejan huérfanos: el índice viejo ocupa espacio y vive en los backups. El índice es derivado (`V12__project_docs.sql:1-5`), así que lo correcto es borrarlo y reconstruirlo, no moverlo.
2. **`register_project` trata «mismo id, otra raíz» como colisión** (`project.rs:83-87`). Con identidad por remoto, dos clones **deben** compartir id y tener raíces distintas. La tabla `project` es `PRIMARY KEY (id)`: una sola `root_path` por id. Hay que separar «el id» de «las raíces donde vive» o el segundo clone daría 409.

### Unir dos scopes sin reconciliar

La operación que esta exploración llama «unir dos ids» (el re-key, `merge`) deja, en el scope unido:

- **Dos versiones vigentes de un tema.** Dos ids legados con filas del mismo agente y el mismo `topic_key` quedan ambas vivas. La próxima escritura de esa clave la arregla (`revise_in` reemplaza todo lo vivo del scope), pero hasta entonces el agente lee las dos.
- **Dos filas vivas con el mismo texto.** Mismo `content_hash` en el mismo (agente, proyecto): el dedup las refuerza al guardar, pero el re-key no pasa por ahí, y ninguna escritura posterior las junta.

Forma del arreglo, dentro de la transacción del re-key: aplicar la regla de `revise_in` a cada clave duplicada (gana la más nueva, causa `revision`) y marcar los duplicados exactos como refuerzo de la fila más vieja. Esos cambios hay que registrarlos para que el re-key siga siendo reversible: hoy `project_rekey` sólo anota movimientos de `project_id`, así que un `status` o `superseded_by` cambiado no se deshace con el SQL de `V9__project.sql:12-16`. Un ayudante «buscar el gemelo vivo dentro de la transacción que escribe» puede compartirse con el del insert del worker, que ya busca así el gemelo y lo refuerza (`insert_unless_twin`, `crates/clawg-core/src/memory/store_insert.rs:429`).

### Qué más depende de una sola raíz

- `search_project` sirve el índice de **una** raíz por id: `root_of` devuelve la `root_path` guardada (`crates/clawg-core/src/memory/project_docs/mod.rs:426-435`) y el refresco la indexa (`mod.rs:221`). Dos clones con ramas distintas tienen documentos distintos. Si comparten id, el índice describe el clone que se registró primero, no el que el agente está editando.

## Lo que ya existe y se reutiliza

- `register_project`: múltiples `legacy_ids` en una transacción, registro por fila en `project_rekey`, idempotencia, `workspace.md` por `adopt_legacy` (`project.rs:56-124`). Rollback documentado en `V9__project.sql:12-16`, con test (`the_rekey_log_restores_every_row_it_moved`, `project.rs:235-261`).
- `project.root_path`: guardado justamente para recomputar ids (comentario de `V9__project.sql:3-5`).
- `root_from_marker`, `parse_gitdir`, `normalize_lexical`: lectura pura de `.git`/`commondir` sin llamar a `git` (`identity.rs:87-126`). Leer `<raíz>/.git/config` sigue el mismo modelo: una función pura que recibe el texto y otra que lo lee.
- Un worktree ya resuelve al repo principal, así que el `.git/config` que se lee es el del principal para todas las ramas.
- Los snapshots: `Reason::PreMigration`, `PreRestore`, `Manual`, `PreReembed` (`crates/clawg-core/src/memory/backup.rs:119-128`) y `MemoryRuntime::snapshot(reason)` (`runtime.rs:195-203`). `register` no toma ninguno hoy (`api.rs:1017-1038`).
- `is_project_id` (`project.rs:40-49`): valida `{nombre}-{4 hex}`; hay que ensancharla si cambia el ancho.
- `clawg backup`, `restore`, `purge`, `sharing`: patrón de subcomando de mantenimiento (`cli.rs:68-122`). No hay `projects`.
- Test de «base de la versión anterior» con ids intactos: `a_database_from_the_previous_version_opens_with_its_project_ids_untouched` (`crates/clawg-core/src/memory/db.rs:677-737`).

## Enfoques posibles

### Identidad

**A. Remoto `origin` normalizado, si no hay remoto la raíz canónica (recomendado).** La identidad de un proyecto es una **clave**: `host/owner/repo` normalizado del remoto `origin`, o la ruta canónica cuando no hay remoto. El id sale del hash de la clave. Los worktrees ya resuelven a la raíz del repo principal, así que no suman casos.

- Normalización pura (función con tests de tabla): quitar esquema (`https://`, `ssh://`, `git://`), usuario y contraseña, el `.git` final y la barra final; pasar `git@host:owner/repo` a `host/owner/repo`; host en minúsculas. Se descartan los remotos que no son un servidor: rutas locales (`/srv/git/app.git`, `../app`, `file://`), que dos clones resolverían distinto.
- **Sin red.** Es lectura de `.git/config`, igual que se lee `.git`/`commondir`.
- A favor: dos clones comparten memoria sin configurar nada. Un repo movido de carpeta mantiene su id mientras el remoto no cambie.
- En contra:
  - El id deja de depender de la ruta, así que **toda raíz con remoto cambia de id** la primera vez y hay que re-keyar (ver «Re-key»).
  - Un fork tiene otro remoto: es otro proyecto (decisión abierta, ver abajo).
  - Cambiar el remoto (`git remote set-url`, repo transferido, migración de GitHub a GitLab) cambia el id.
  - Usuarios que **quieren** memorias separadas por clone.

**B. Sólo la clave de la raíz git común (lo de hoy), más `projects merge`.** El usuario une ids a mano.

- A favor: sin cambio de identidad, sin migración. Reversible por construcción.
- En contra: nada es automático; cada clone nuevo parte la memoria hasta que el usuario lo nota. Para eso no hace falta una exploración nueva.

**C. Clave explícita en el repo.** Un archivo `.clawg` o una clave en `.git/config` (`clawg.project = nombre`) que el usuario fija.

- A favor: ningún falso positivo.
- En contra: fricción cero hoy; con C hay que configurar cada clone, y el problema (olvidar) es el mismo que el actual. Sirve como **override** de A, no como base.

**Recomendación:** A como base, B como red de seguridad (`merge`) y C como override. Los tres se apoyan en la misma clave.

### Re-key

Se pasa de «el id lo calcula el puente de su ruta» a «el id lo calcula de una clave». Opciones sobre **quién** sabe de los ids viejos:

**R1. Que el daemon los busque en `project` (recomendado).** El puente envía la raíz; el daemon calcula la clave nueva y toma como ids viejos: el id de la ruta cruda, el id derivado de cada `root_path` guardada que comparta la clave, y el id que `project` ya tenía para esa raíz. Reusa `register_project` con una lista más larga de `legacy_ids`. Hace falta que `project` guarde la clave (columna nueva o tabla `project_root(id, root_path)`).

- Para un clone cuyo disco ya no existe, la clave no se puede recomputar leyendo `.git/config`. Por eso hay que **guardar la clave al registrar**, desde la próxima versión; los proyectos ya registrados no la tienen y se completan al abrirlos.
- Los proyectos de clones que nunca se vuelven a abrir se unen con `merge` (abajo).

**R2. Que el puente mande todos los ids posibles.** No puede: el puente no conoce los otros clones.

**R3. Migración SQL que recomputa todo.** No se puede: la clave sale del filesystem, no de la base.

Como no puede ser una migración de esquema que recalcule, el re-key ocurre **en tiempo de ejecución** (como hoy, al registrar). Eso obliga a un snapshot previo, que `register` no toma: se necesita un `Reason` nuevo o llamar a `Manual` antes del primer re-key de la sesión. La regla de `config.yaml` para ids guardados (probar con datos de la versión anterior) aplica de lleno.

### Ancho del hash

El id es hoy 16 bits y sólo choca entre carpetas homónimas (`identity.rs:11-15`; ver `project-root-guard`).

- **W0. No ensanchar** (sólo válido si 29 degrada en vez de bloquear; ver «Condición con `project-root-guard`» más abajo). El 409 de `project-root-guard` hace visible el choque; con la clave guardada en `project`, la comparación pasa de «misma `root_path`» a «misma clave», que es la correcta.
- **W1. Ensanchar todo a 64 bits.** Re-key de **todos** los proyectos, también los que no tienen remoto. Relaja `is_project_id` (`project.rs:40-49`) y acepta dos anchos mientras haya filas viejas. Costo: una operación masiva sobre cada tabla de la tabla anterior, un snapshot y más superficie de error.
- **W2. Ensanchar sólo los ids nuevos (los de remoto).** Un proyecto sin remoto conserva su id de 16 bits; uno con remoto sale con 64. No re-keya a quien no necesita cambiar de id, y cada id nuevo nace sin el problema.

- **W3. El registro asigna el id; el nombre de carpeta es sólo una etiqueta.** `project` ya persiste raíz → proyecto y es la única fuente de qué raíz tiene qué id: `register_project` hace `INSERT OR IGNORE` y compara la `root_path` guardada (`project.rs:79-87`, `:113`), y la tabla es `PRIMARY KEY (id)` (`V9__project.sql:7`). Con W3 el id deja de ser función pura de la ruta: la primera vez que se ve una raíz, el daemon le asigna un id libre (`{nombre}-{4 hex}` si está libre, otro sufijo libre si no) y lo guarda junto a la raíz; las siguientes veces lo busca. Un choque deja de existir **por construcción**, porque dos raíces nunca comparten fila. Hoy el daemon ya deriva el id de `root_path` y no lo toma de quien llama (`api.rs:1015-1016`, `project.rs:65`) y ya lo devuelve al puente (`RegisteredBody { project_id, rekeyed }`, `api.rs:1037`; `DaemonClient::register_project` lo lee, `crates/clawg-cli/src/client.rs:1309-1328`).
  - A favor: elimina el 409 por colisión de hash sin ensanchar nada (sirve también a raíces sin remoto); conserva el formato `{nombre}-{4 hex}` y `is_project_id` (`project.rs:40-49`) sin cambios; no re-keya a quien no choca.
  - En contra:
    - El puente calcula el id **localmente** hoy (`derive_project_id`, `identity.rs:11-15`; `hash4` es SipHash-1-3, 16 bits bajos, `identity.rs:146-153`) y el id legado se deriva igual. `project_for` descarta el id que devuelve el daemon y usa el calculado (`crates/clawg-cli/src/mcp/server.rs:873-886`). Con W3 el puente necesita el id del daemon antes de cada llamada: hay que decidir qué hace si el daemon no responde o es una versión anterior (hoy un fallo de registro sólo es un warn y sigue con el id calculado, `server.rs:873-878`), y el id calculado localmente deja de ser autoritativo en el caso de choque.
    - Hace falta una tabla o un índice único por raíz (hoy `root_path` no es único) y una migración con test sembrado en la versión anterior; los ids existentes que ya chocan no se pueden separar (ninguna fila guarda su raíz de origen, ver 29), y los que no chocan se conservan tal cual.
    - El id deja de ser reproducible desde la ruta: si se pierde `memory.db` y se vuelve a abrir en otro orden, un id asignado por orden de llegada puede invertirse (el mismo motivo por el que 29 descartó una sal por orden de llegada). Sólo se mitiga con el snapshot y porque el mapa vive en la misma base que las filas.
    - Cualquier re-key de ids existentes pasa por `project_rekey` y un snapshot previo (ver Riesgos).

**Recomendación:** W2 si el usuario quiere cerrar el problema de colisión; W0 si prefiere no tocar más. W1 no se recomienda: re-keyar todo sin motivo funcional contradice la regla de no tocar lo que funciona. La decisión depende de cuánto pese que un choque entre dos remotos con el mismo nombre de repo (`acme/app` y `me/app`) bloquee a alguien: 1 en 65.536. W3 se suma como la alternativa que resuelve el choque sin ensanchar el id, a cambio de que el puente dependa del daemon para conocerlo; se pesa contra W2 (id más ancho derivado de la clave, el puente lo sigue calculando solo). **Esta recomendación queda condicionada a lo que elija `project-root-guard` (29), ver abajo.**

#### Condición con `project-root-guard` (29)

29 recomienda fallar cerrado ante un 409 (no guardar ni buscar en esa raíz) y remite a este cambio para el remedio. Pero con W0 este cambio conserva 16 bits: dos repos genuinamente distintos con la misma carpeta (`app`, `web`) seguirían chocando, y el usuario quedaría bloqueado sin remedio. Por eso:

- Si 29 elige fallar cerrado (su opción A), este cambio **debe** entregar W2 o W3 (o 29 debe adoptar su alternativa de degradar a sin proyecto, opción B). W0 deja de ser una opción válida en ese caso, y la recomendación «W2 si el usuario quiere cerrar el problema; W0 si prefiere no tocar más» pasa a ser «W2 o W3»; W0 sólo se sostiene si 29 degrada en lugar de bloquear.
- W2 sólo ensancha los ids que salen de una clave de remoto: un par de repos **sin remoto** con la misma carpeta sigue en 16 bits y seguiría bloqueado bajo la opción A de 29. W3 cubre también ese caso; W2 no. Es el argumento principal a favor de W3 si 29 falla cerrado.

### Índice de documentos con clones

- **I1. El índice sigue la raíz (por checkout).** El índice de documentos se queda en el id de ruta; la memoria usa el id de la clave. Dos ids por sesión, y el puente decide cuál usar en cada herramienta (`search_project` usa el de ruta). Es lo correcto: los documentos describen un árbol de trabajo (una rama) y la memoria describe un proyecto.
- **I2. El índice sigue el proyecto, con la raíz más reciente.** `root_of` devuelve la última raíz registrada. Dos clones con ramas distintas se pisan: cada cambio de sesión reindexa (el sha256 de los archivos cambia, `corpus.rs`, `mod.rs` `Refreshed`), y mientras tanto el otro clone ve documentos de una rama ajena.

**Recomendación:** I1. Cuesta un segundo id en el puente, pero evita servir documentos de la rama equivocada. Es la parte que puede hacer que el cambio deje de ser M.

### `clawg projects merge <desde> <hasta>`

Une dos ids que el usuario sabe que son el mismo proyecto (clone sin remoto, repo movido, remoto cambiado). Reusa `register_project`: mueve las filas de `desde` a `hasta` con registro en `project_rekey`, soporta las tres tablas, deja los ids viejos listados.

- Debe ser **reversible**: el registro por fila ya permite deshacerlo con el SQL de `V9__project.sql:12-16`. Una cadena de merges se deshace paso a paso (la condición `r.project_id = tabla.project_id` sólo mueve las filas que hoy están bajo ese id, y `project_rekey` no es única por fila: `V9__project.sql:26`). Un comando `unmerge` o el SQL documentado hacen falta.
- Un daemon corriendo: lo atiende el daemon, como `clawg backup` y `clawg sharing`.
- Necesita snapshot previo.
- Borra o reconstruye el índice de documentos del `desde` (derivado).
- Un merge entre ids de distinta clave no es una colisión: es una orden explícita del usuario.

### Opt-out

Hay quien quiere memorias separadas por clone (por ejemplo, un clone de producción y otro de experimentos). Opciones: clave explícita C por clone (override) o un `clawg projects split`. C ya lo cubre, porque pone otra clave a ese clone.

### Reconciliación al unir

**Q1. Reconciliar dentro de la transacción del re-key (recomendado).** Ver «Unir dos scopes sin reconciliar»; el costo es registrar lo que cambia además del `project_id`.
**Q2. Dejarlo al `doctor-checks` (31).** Detecta las dos invariantes después; no las arregla y el duplicado exacto nunca se corrige solo.
**Q3. Rechazar la unión si hay claves o hashes repetidos.** Es seguro, pero bloquea justo el caso de dos clones con memorias parecidas, que es el motivo del cambio.

## Descartado

- **Identidad por nombre del repo:** repos distintos con el mismo nombre (`app`, `api`, `web`) se fusionarían sin que nadie lo decida. La clave tiene que incluir host y dueño.
- **Identidad por hash de commits, por ejemplo el primer commit:** necesita recorrer el historial (`git` o una librería), es lento en repos grandes, no existe en repos recién creados, y dos repos creados desde la misma plantilla comparten el primer commit sin ser el mismo proyecto.
- **Consultar el remoto por red** (`git ls-remote`, API de GitHub): el daemon es local y funciona sin red; un id que cambia según la conectividad es peor que ninguno.
- **Llamar a `git rev-parse --git-common-dir`** en producción: lo que ese comando da para un worktree ya lo lee `root_from_marker` sin un subproceso, que además hay que lanzar con cuidado en Windows (`crates/clawg-cli/src/detach.rs`). No se agrega dependencia ni proceso.
- **Usar cualquier remoto si no hay `origin`:** `upstream`, `fork` y demás son la excepción en un clone con varios remotos, y elegir el primero depende del orden del archivo. O `origin` o nada.
- **Ensanchar en 29:** re-keya todo, necesita snapshot y migración de ids; 29 es S y sin esquema.
- **Auto-fusión por parecido de nombre de carpeta:** el 409 ya existe porque ese camino fusiona proyectos ajenos.
- **Guardar la URL completa:** puede contener credenciales (`https://usuario:token@host/...`). Sólo se guarda la clave normalizada.

## Alcance candidato

1. Función pura `normalize_remote(&str) -> Option<String>` (tabla de casos: https, ssh, scp-like, con puerto, con usuario y token, `.git`, mayúsculas, rutas locales) y `origin_of(config_text) -> Option<String>` (parser mínimo de `[remote "origin"] url`).
2. `ProjectIdentity` con una clave (`remote` o raíz) además de `id`, `root` y `legacy_id`. El hash se calcula de la clave.
3. Esquema: guardar la clave y permitir varias raíces por id (`project_root` o columna), y relajar «misma raíz» a «misma clave» en `register_project` (`project.rs:83-87`). Migración con test de base sembrada en la versión anterior.
4. El daemon reúne los ids viejos de una raíz (`R1`) y los pasa a `register_project`. Snapshot previo al primer re-key de una raíz.
5. Limpieza del índice de documentos y de `synthesis_review` del id viejo (derivados o reconstruibles).
6. `clawg projects merge <desde> <hasta>` y su deshacer; `clawg projects list` (compartido con 29).
6b. Reconciliación en la transacción del re-key y de `merge` (Q1): `revise_in` por clave repetida, refuerzo de duplicados exactos y un registro de esos cambios que mantenga reversible el re-key.
7. Override explícito (C) si se decide.
8. Documentar en `CLAUDE.md` la identidad por remoto, la migración y el comando.

### No-alcance

- Mirar `.git` más allá del remoto (ramas, historial).
- Unir proyectos por contenido o parecido.
- Red.
- Dividir un proyecto ya fusionado por filas (el registro de `project_rekey` lo permite paso a paso, pero no hay herramienta para elegir qué fila).
- Memoria compartida entre usuarios por remoto: el almacén es local.
- Cambios de ranking o de recuperación.
- Permisos por proyecto en `sharing.json`.
- El chequeo de colisiones en reposo (`doctor-checks`).
- Tocar worktrees de repos bare: se mantiene la decisión actual (`identity.rs:92-95`).

## Decisiones abiertas

Son del usuario.

- [ ] **Qué identifica un proyecto.** ¿Remoto `origin` normalizado y raíz si no hay (A, recomendado), sólo raíz con `merge` manual (B), o con clave explícita como override (C)?
- [ ] **Forks.** Un fork tiene otro `origin`, así que es otro proyecto. ¿Se acepta, o el usuario espera que el fork comparta con el original (entonces haría falta mirar `upstream`)?
- [ ] **Qué remoto.** ¿Sólo `origin` (recomendado) o el primero que exista?
- [ ] **Mayúsculas en el dueño y el repo.** GitHub las ignora; otros servidores no. ¿Se pasa todo a minúsculas o sólo el host?
- [ ] **Si cambia el remoto.** Con R1 el id nuevo recoge las filas del viejo si `project` recuerda la raíz. ¿Se hace solo, o `merge` manual?
- [ ] **Ancho del hash.** W0, W1, W2 o W3. Decide el usuario porque el costo es un re-key. Depende de la colisión que elija 29: si falla cerrado, W0 queda fuera (ver «Condición con `project-root-guard`»).
- [ ] **Índice de documentos.** I1 (por checkout) o I2 (por proyecto).
- [ ] **Opt-out.** ¿Hay clave explícita por clone, o basta `merge`/ningún comando?
- [ ] **Comando de unión.** Nombre y forma: `clawg projects merge`, y si lleva `--dry-run` que cuente filas sin mover nada.
- [ ] **Repos sin remoto que luego lo ganan.** Al hacer `git remote add origin`, ¿el id pasa a la clave de remoto y se re-keya, o se queda en el de la ruta?
- [ ] **Reconciliación al unir.** ¿Q1 (reconciliar en la transacción), Q2 (sólo detectar) o Q3 (rechazar)? Y dónde se registra lo que cambia para poder deshacerlo (una columna o tabla nueva junto a `project_rekey`).
- [ ] **Snapshot.** ¿Un `Reason` nuevo (`PreRekey`) o reutilizar `Manual`?

## Cómo se mide

Tests RED primero:

1. `normalize_remote`: `https://github.com/Acme/App.git`, `git@github.com:Acme/App.git`, `ssh://git@github.com:22/Acme/App` y `https://u:tok@github.com/Acme/App/` dan la misma clave; el token no aparece en la salida. Rutas locales y `file://` dan `None`.
2. `origin_of` lee `[remote "origin"]` entre otras secciones y devuelve `None` sin `origin`.
3. Dos carpetas con `.git/config` y el mismo `origin` resuelven al mismo `id`; con otro `origin`, a otro. Sin remoto, el id es el de hoy (el test de `project_root.rs` no cambia).
4. Un worktree sigue resolviendo al repo principal (`project_root.rs:38-50`) y comparte clave con él.
5. **Base sembrada en la versión anterior:** filas en tres tablas bajo ids de ruta de dos clones del mismo remoto; al registrar el segundo, todas quedan bajo un id, ninguna fila se pierde, `project_rekey` las lista, y la búsqueda léxica y la vectorial las alcanzan. Es la regla de `config.yaml` para `project_id`.
6. Rollback: el SQL de `V9__project.sql:12-16` deja cada fila en su id anterior (incluidas las borradas lógicamente).
7. Idempotencia: registrar dos veces no mueve nada.
8. Mismo id, dos raíces con la misma clave: no es 409. Mismo id con claves distintas: sí es 409 (la colisión real).
9. Hay snapshot antes del primer re-key y no hay otro en el segundo registro.
10. `workspace.md` del clone viejo se adopta bajo el id nuevo.
11. Índice de documentos y `synthesis_review` del id viejo se limpian; el índice del id nuevo se reconstruye.
12. `merge`: une, registra, se deshace; con `--dry-run` no cambia nada; un id inexistente falla sin tocar nada.
13. Clones con ramas distintas (si I1): `search_project` sirve los documentos del checkout activo.
14. No se promete que el binario anterior abra la base nueva: una base de una versión posterior se rechaza intacta (`CLAUDE.md`, perfiles de datos). El test confirma que ese rechazo sigue en pie con la migración nueva.

15. **Reconciliar al unir:** dos ids legados con el mismo `topic_key` y el mismo texto; tras registrar, hay un save vivo por clave y una fila viva por `content_hash` en el scope. Hoy fallan ambas. El deshacer devuelve cada fila a su id y a su estado anterior.

`rag-eval --diff`: no es requisito para identidad. Sí conviene correr `rag-eval --real` una vez si el re-key toca el conjunto dorado, para confirmar que no cambia ninguna métrica (el golden usa un proyecto fijo).

## Riesgos

- **Re-key a escala:** mover filas de cada proyecto con remoto, en tiempo de ejecución y sin migración, es el riesgo mayor. Mitiga el snapshot previo, el registro por fila y el test 5. Mientras `register` no tome snapshot (hoy no lo hace), no hay red.
- **Poda de snapshots (`PreRekey`):** un `Reason` automático nuevo comparte el presupuesto `AUTO_KEEP = 5` con `pre-migration` y los demás (`is_automatic` es «todo menos `Manual`», `crates/clawg-core/src/memory/backup.rs:143-145`; `to_prune` conserva los 5 más nuevos de todas las razones juntas, `backup.rs:227-243`). Cada re-key sube la chance de que se pode el snapshot `pre-migration` que protege la migración de esquema de este mismo cambio. La mejora pendiente del backlog (conservar el más nuevo de cada razón) debe aterrizar **antes** de este cambio, o `PreRekey` va fuera de la poda automática.
- **Fusión no deseada:** dos clones con contenidos de memoria que el usuario quería separados quedan juntos, y separarlos fila a fila no tiene herramienta. Mitiga C y que el primer re-key sea visible (log, `clawg projects list`).
- **Un id que cambia con el remoto:** cambiar el `origin` mueve la identidad. Sin R1 queda huérfana la memoria vieja.
- **Re-key que deja duplicados:** sin reconciliar, dos versiones vigentes de un tema (la próxima escritura de la clave las arregla) o dos filas vivas idénticas (nunca se arreglan). Mitiga Q1 y el test 15. El mismo defecto lo tendría `merge`.
- **Credenciales en la URL:** la normalización tiene que descartarlas antes de guardar o hashear. Test 1.
- **Remotos que no son servidores** (rutas, `file://`, espejos locales): dos clones darían claves distintas o iguales por azar; se descartan.
- **Índice de documentos:** con I2, documentos de la rama equivocada; con I1, un segundo id y más código en el puente.
- **Esquema:** una migración y un `PRIMARY KEY (id)` en `project` que hay que cambiar sin perder las raíces registradas; una base de una versión posterior se rechaza intacta, así que la vuelta atrás exige restaurar el snapshot.
- **Tamaño:** M si I1 y W2 entran; si se complica, partir en identidad y `merge`.
