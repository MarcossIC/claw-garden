# project-registry — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 | M–L (un arreglo de integridad S, el registro con localizadores M, unión y proyectos sin carpeta M) | project-reassign (77) y project-clone-identity (30), archivados: se construye sobre sus R1–R19; se coordina con ranking-at-scale (86) e import-job (87) | exploración (borrador) |

**Tesis:** hoy un proyecto *es* una carpeta. Su id sale del camino (`derive_project_id`, `{basename}-{hash4(path)}`, `crates/clawg-core/src/memory/identity.rs:11`), `project.root_path` es `NOT NULL` (`crates/clawg-core/migrations/V9__project.sql:6-10`) y todo lo que se guarda, se busca, se mueve o se importa pasa por una raíz. De ahí salen cuatro fallos distintos: (1) **un bug de integridad**: un repositorio distinto clonado en el camino donde vivía otro se funde con él cuando su remoto nunca se registró (`plan_in`, `project.rs:191-195`), y el remoto del intruso queda atado al proyecto ajeno (`write_in`, `project.rs:253-257`); (2) una carpeta sin `origin` que se mueve o se renombra pierde su proyecto; (3) no existe un proyecto sin carpeta (una idea, notas sobre un binario o una planilla); (4) no hay un lugar donde detectar y unir un proyecto duplicado. El cambio separa **identidad** de **ubicación**: un registro de proyectos (id estable, nombre, lápida de unión) y una tabla de localizadores (camino, remoto, primer commit) que apuntan a él, con la regla de que un localizador solo vincula si lo valida una huella independiente del camino. Los ids existentes no se reescriben, ninguna fila de `memory` cambia de `project_id`, y unir dos proyectos reutiliza el motor de movimientos que ya se puede deshacer. El arreglo del bug (1) puede salir antes, solo, porque no necesita tablas nuevas para el caso común.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El id sale de `{basename}-{hash4(path)}` y los choques de 16 bits se resuelven con candidatos | **Cierta.** `derive_project_id` (`identity.rs:11-15`), `candidate_id` con `#n` para n ≥ 1 (`:23-30`), `CANDIDATES` = 64 (`:18`), `is_candidate_for` (`:33-35`). Con 4 hex hay 65.536 ids por nombre; `root_id_in` toma el primero libre (`project.rs:119-131`) y falla con 409 `no_free_project_id` si los 64 están tomados. |
| `project` es una tabla de raíces, `project_remote` junta los clones de un remoto | **Cierta.** `project(id, root_path NOT NULL, created_at)` (V9:6-10), `index_docs` en V13 (`V13__project_index_docs.sql:6`), `project_remote(remote PK, project_id, created_at)` (V24 `:9-13`) con el id de proyecto de la primera raíz que trajo el remoto. `idx_project_root` no es único (V24:35): dos filas pueden compartir camino. |
| **Bug:** otro repo clonado en el camino de A se funde con A | **Cierta, y más amplia que el enunciado.** Con `assigned`: `root_id_in` devuelve el id *grabado para ese camino* (`project.rs:119-123`, `recorded_id_in` `:103-106`, sin mirar qué hay hoy en esa carpeta); `plan_in` usa el proyecto del remoto solo si `project_remote` ya conoce el remoto del repo B y, si no, `project_id = root_id` (`:191-195`), o sea el de A. Después `write_in` hace `INSERT OR IGNORE INTO project_remote (B, proyecto de A)` (`:253-257`), así que **todo clone posterior de B se une a A**. Pasa tanto si A tenía remoto (otro distinto) como si no. Sin `assigned` (cliente anterior): `derive_project_id(path)` da el mismo id y el 409 `project_root_collision` solo salta si ese id lo tiene *otra* raíz o la raíz tiene otro id (`:196-206`); el mismo camino no choca consigo mismo, así que el fundido es el mismo. El camino de solo lectura (`project_id_in`, `:136-145`) y el planificador de importación (`plan_ids_in`, `:151-185`, mismo `recorded_id_in`, `:156`) tienen la misma regla. No hay ninguna validación de que la carpeta siga siendo el mismo repo. |
| Una carpeta sin `origin` que se mueve o se renombra recibe otro id | **Cierta.** El camino nuevo no está grabado, `root_id_in` toma `candidate_id(raíz_nueva, 0)` (`project.rs:124-128`), y `legacy_ids` solo cubre el id del camino *por el que se llegó* a la misma raíz (symlink, subcarpeta; `identity.rs:54-58`). Sus memorias quedan bajo el id viejo. Con `origin`, el remoto sí las reúne (R16). |
| Un proyecto sin carpeta no puede existir | **Parcialmente falsa.** Hoy existe como *etiqueta*: `save_memory` con un `project_path` que no existe pasa por `resolve_project_root`, que normaliza el texto si no puede canonicalizar (`identity.rs:67-74`), y `ids_for` registra esa raíz sin mirar el disco (`crates/clawg-cli/src/mcp/server.rs:863-893`); el daemon solo exige directorio con `index_docs` (`daemon/api.rs:2024-2026`). Lo que sí exige directorio: `clawg projects add` (`projects.rs:31`), `clawg projects move --to` (`:49-50`) y `update_memory.to_project_path` (`server.rs:1152-1158`); `--to-project` exige un proyecto registrado (`relocate.rs:424-435`). Entonces la etiqueta existe, pero su identidad es el hash de un camino inventado, no se puede nombrar por nombre y nada la distingue de un camino real. |
| La búsqueda de un proyecto también lee `project_id IS NULL` y todo `user_preference` | **Cierta.** `memory_project_predicate!` (`crates/clawg-core/src/memory/search.rs:107-113`): `param IS NULL OR project_id = param OR project_id IS NULL OR memory_type = 'user_preference'`. Una fila sin proyecto compite en todos los proyectos. |
| Quedan 15.709 de 54.436 filas importadas sin proyecto y colocarlas exige una carpeta | **Sin verificar en el repo; la mitad operativa es cierta.** La cifra viene del store de producción y no se leyó (`~/.clawg` no se toca). Que colocar requiere carpeta es cierto para `projects move --to` y para `update_memory` (arriba); `--to-project ID` exige que el id exista en `project`, que solo se crea desde una raíz (`relocate.rs:430`). Las notas de un proyecto de la fuente sin carpeta local no se pueden mapear con `--project-map NAME=PATH` sin inventar un camino (`import.rs:1512-1551`, el destino es una raíz). |
| Reimportar un export de clawg tras mover filas entre proyectos las inserta de nuevo | **Cierta, con matiz sobre qué se puede arreglar.** `PRESENT_SQL` busca `agent_id IS ?1 AND COALESCE(project_id,'') = COALESCE(?2,'') AND content_hash AND created_at` (`portability/import.rs:1130-1133`); `TWIN_SQL` y `TOPIC_HOLDERS_SQL` también por proyecto (`:1138-1150`). El proyecto de la fila exportada se traduce por raíz (`roots_here`, `:226-243`; `project_ids`, `:299-345`), así que una fila movida ya no está «presente» bajo su proyecto viejo. Para una fila de otra herramienta (`source = 'import'`) la presencia se halla por origen donde esté ahora (`.claude/CLAUDE.md`, `clawg import`), no para un export de clawg. |
| Existe una clave de proyecto independiente del camino | **Cierta pero incompleta.** `origin_key` (`identity.rs:175`) normaliza `origin` a `host/path`. Un repo sin `origin` no tiene nada; no hay en el código ninguna lectura del primer commit (`rg root_commit|rev-list` no da resultados), así que esa huella es nueva. |
| La sesión del puente y los hooks usan el id que da el daemon | **Cierta, y es una fuente de ids obsoletos.** El puente guarda el id por `(raíz, legacy_id)` toda la vida de la sesión (`server.rs:874-891`); si el registro falla, escribe bajo el id derivado sin registrarlo (`:885-891`). Una unión de proyectos hecha mientras hay un puente abierto no le llega: seguiría escribiendo bajo el id absorbido. |

## Problema (verificado contra el código)

### 1. Mapa de dónde se deriva, resuelve, guarda y consume un id de proyecto

**Se deriva o asigna**
- `derive_project_id`, `candidate_id`, `ProjectIdentity::resolve` (`identity.rs:11-59`); `resolve_project_root` (`:67-79`).
- `root_id_in` / `plan_ids_in` / `project_id_in` / `plan_in` (`project.rs:119-231`).
- Cliente anterior sin `assigned`: `derive_project_id` en `plan_in` (`:196-206`), el puente cae a `project.id` si el registro falla (`server.rs:885-891`), el hook cae a `identity.id` (`hook.rs:221-228`).

**Se resuelve (camino a id)**
- `POST /v1/projects`: `register` (`daemon/api.rs:2015-2060`) → `register_project` (`project.rs:300-345`) → `Registration { assigned, remote, legacy_ids }`.
- Puente: `ids_for`/`project_for`/`save_project` (`server.rs:855-920`), `project_path` en `save_memory`, `search_memory`, `recall_context`, `search_project`, `list_memories`, `to_project_path` en `update_memory` (`server.rs:170-326`, `:1151`).
- Hooks: `project_of` (`hook.rs:217-228`), una resolución en cada evento, con plazo de 3 s.
- CLI: `projects add` (`projects.rs:30-35`), `projects move --to` (`:49-54`), `--here` (`:59-60`), `memories --here` (`memories.rs:435-436`, lee sin escribir), `export --project` (`portability/export.rs:81-93`, `project_id_of`), `main.rs:226-227`.
- Importación: `roots_here`/`plan_ids_in` → `ProjectHere` (`import.rs:226-243`, `:299-345`), `--map-root` (`:75`, `:236-240`), `--project-map` (`:77`, `:1512-1551`).

**Se guarda**
- `memory.project_id` (V1:9, con el CHECK de alcance V1:32-33; índices V1:41-42, V6, V16, V18, V20, V25) y su copia en `memory_fts` (V8:39, `:64-65`, `:76-77`, `:86-87`; los triggers la siguen, por eso una mudanza no toca el índice).
- `episodic_trajectory.project_id` y `procedural_skill.project_id` (V1:66, V1:117), re-keyadas con `project_rekey` (V9:19-29, `OTHER_PROJECT_TABLES`, `project.rs:27`).
- `project` (id = id de *raíz*, no de proyecto), `project_remote` (V24).
- `memory_move.from_project/to_project` (V21:14,16) y `memory_move_supersede` (V24:19-26).
- Índice de documentos: `project_doc*.project_id` (V12:11,24,37,47) **con el id de raíz**, no el de proyecto (`project_docs/mod.rs:505`, `server.rs:1235` usa `ids.root`).
- `synthesis_review.scope_key` (V10:22-28) = id de proyecto o `''`.
- Diario de importación: `memory_import_extra` con `kind` `project`/`remote` guarda ids y claves (V26:60; `import.rs:1203-1215`).
- Sesiones registradas por `(agente, proyecto)` en memoria del daemon (`daemon/sessions.rs:29-44`, `api.rs:1341`, `:1585`).
- Archivos: `workspace.md` por `(agente, id de proyecto)` (`workspace.rs:44-48`, `adopt_legacy` `:57`), y el evento del outbox lleva el `project_id` del save.
- No es un id: `memory.origin_project` (V23) es el nombre de proyecto en otra herramienta.

**Se consume**
- Lectura: `memory_project_predicate!` (`search.rs:107-113`), `list`, `LAST_SESSION_SQL`, RECENT, `synthesize` (`synthesis.rs:114`, `:157`), disputas, el workspace (`workspace.rs:287`, `:339`).
- Escritura: guardado, dedup, `topic_key` por proyecto (V16/V20), detector, revival (V17:28).
- Mover: `check_target_in` (`relocate.rs:424-435`), `move_memories_core` (`:607`), `undo_move_core` (`:632`), `merge_in` (`:884`); API `POST /v1/memories/move` (`api.rs:1720`), `PATCH` con `project_id`/`no_project` (`api.rs:459`, `:515-526`).
- Diagnóstico: `doctor::projects` (`doctor/checks.rs:563-585`) compara cada fila con `is_candidate_for(root, id)`; un proyecto sin raíz o un id no derivado del camino saldría como `id_mismatch`.

**No hay clave foránea** de ninguna de esas columnas hacia `project.id`. SQLite no puede agregar una a una tabla existente sin reconstruirla, y `memory` tiene la tabla virtual vec0, el índice FTS y sus triggers (V8): reconstruirla para esto es el cambio más riesgoso del repositorio. Hoy tampoco hay garantía de que cada `project_id` de `memory` exista en `project`: el puente escribe bajo un id derivado cuando el registro falla (`server.rs:885-891`) y una importación deja fuera ids tomados (`import.rs:1170-1181`).

### 2. Lo que falla, ordenado por gravedad

1. **Fundido de repos distintos** (premisa del bug). Es pérdida de aislamiento de datos: las memorias de B se leen y se escriben como si fueran de A, y el mal se propaga por `project_remote`. Ningún test del repo cubre «mismo camino, otro repo» (los tests de R16 usan dos caminos y un mismo remoto).
2. **Mudanza de una carpeta sin remoto**: memorias huérfanas bajo el id viejo, sin aviso. `doctor` no lo ve: el id viejo sigue siendo un candidato válido de su raíz vieja (`checks.rs:563-585`).
3. **Sin identidad que sobreviva a la carpeta**: borrar la carpeta no borra el proyecto (la fila de `project` queda), pero tampoco hay forma de nombrarlo ni de mover su contenido a otra ubicación sin crear una raíz.
4. **Sin unión explícita**: `merge_in` existe pero solo lo dispara el registro por remoto o por ids viejos (`project.rs:261`); el usuario no puede decir «A y B son el mismo». Un duplicado (clone accidental, nota mal puesta) se arregla hoy a mano con `projects move` por filtros, que no mueve `synthesis_review`, `workspace.md` ni las tablas `episodic_trajectory`/`procedural_skill`.
5. **Filas sin proyecto que contaminan todas las búsquedas** (`search.rs:107-113`), y no se pueden colocar sin inventar una carpeta. Interactúa con ranking-at-scale (ver Interacciones).
6. **Reimportación tras mover** duplica filas (`import.rs:1130-1150`).

## Lo que ya existe y se reutiliza

- El motor de movimientos: `merge_in` (`relocate.rs:884`) deja una fila viva por clave y por texto, registra `memory_move` y `memory_move_supersede` y se deshace con `projects move --undo N` (R8–R12, R17). La unión de proyectos es el mismo motor con otro disparador.
- `project_rekey` para las otras dos tablas (V9:19-29) y su SQL de vuelta.
- Los snapshots `pre-rekey`/`pre-move` y el patrón «decidir, snapshot, volver a decidir» de `register_project` (`project.rs:311-338`).
- `origin_key` y `normalize_remote` (R14), puros y probados.
- `Reader`/`sharing.json` para el alcance de lectura: no cambia.
- El formato de export v1: un lector ignora los campos que no conoce (`.claude/CLAUDE.md`), así que la línea `project` puede ganar campos opcionales sin subir la versión.
- El diario de importación (V26) para deshacer lo que una importación registre (raíces y remotos hoy; localizadores mañana).
- `doctor` y su forma de hallazgos (`check_id`, `reason_code`, `safe_next_step`).
- La lectura sin escribir (`project_id_in`, `project_id_of`): el planificador puro de la unión debe seguir siéndolo.

## Enfoques posibles

### A. Identidad separada de la ubicación (registro + localizadores)

**Esquema (V27, aditivo).**
- `project_identity(id PK, name, created_at, merged_into NULL, merged_at NULL)`: una fila por cada id de proyecto. `merged_into` es una lápida.
- `project_locator(kind CHECK IN ('path','remote','root_commit'), value, project_id, first_seen, last_seen, unbound_at NULL)` con un índice único parcial sobre `(kind, value) WHERE unbound_at IS NULL`: un remoto o un camino apunta a un solo proyecto a la vez, y la historia se queda como filas con `unbound_at`. Cero localizadores es válido (proyecto sin carpeta).
- La tabla `project` **no se toca**: sigue siendo la tabla de *raíces* (id de raíz, camino, `index_docs`) y el índice de docs sigue colgado de ella, porque se deriva del disco. `project_remote` se conserva y se mantiene como vista de compatibilidad del localizador `remote` para clientes y pruebas actuales (o se reemplaza, ver decisiones).
- Relleno: una identidad por cada id distinto que aparezca en `project.id`, `project_remote.project_id`, `memory.project_id`, `episodic_trajectory`, `procedural_skill`, `memory_move`, `project_rekey` y `synthesis_review` (así ningún id existente queda sin identidad, ni los que hoy no están en `project`); `name` = la parte antes del último `-`; localizadores `path` desde `project`, resueltos al proyecto que hoy leen (el del remoto si lo hay), y `remote` desde `project_remote`. **Ninguna fila de `memory` cambia.**

**Ids.** Los proyectos nacidos de una carpeta conservan el id de hoy (el `candidate_id` de la primera raíz): sin migración de datos y un cliente anterior que deriva el id del camino sigue entendiéndose. Un proyecto sin carpeta recibe `{slug}-{4 hex}` con la misma forma que `is_project_id` (`project.rs:78-87`), comprobando que esté libre, de modo que ningún consumidor (workspace, `legacy_ids`, `--to-project`) necesita una rama nueva. La contra: un id con forma de hash de camino no es opaco, y 16 bits siguen siendo pocos (se resuelve comprobando contra el registro, como `root_id_in`). La alternativa es un id opaco largo para los nuevos, a costa de relajar `is_project_id` y tocar `workspace` y los clientes viejos.

**Resolución desde un `cwd`** (función pura sobre lo leído, `resolve_in(ubicación, huellas, registro) -> Decisión`):
1. Localizador `path` vigente → proyecto X, **solo si la huella no lo contradice**: el `origin_key` actual de la raíz, si existe, no es distinto de los localizadores `remote` de X; y el primer commit actual, si se pudo leer, no es distinto del localizador `root_commit` de X. Sin huella contraria, o si X no tiene ninguna, vale el camino (el caso común: el usuario le agrega un remoto a su repo local).
2. Si el camino contradice: el localizador `path` se cierra (`unbound_at`), y se sigue con 3.
3. Localizador `remote` → su proyecto (R16, sin cambios).
4. Localizador `root_commit` → se **sugiere**, nunca se une solo (un fork o una copia comparten el primer commit). Excepción acotada: si el camino viejo de ese proyecto ya no existe en disco, es una mudanza y se vincula (con aviso en el log y `doctor`); si existe, se pregunta con `clawg projects` (decisión abierta).
5. Nada coincide → proyecto nuevo con su identidad y sus localizadores.

El arreglo del bug es el paso 2: el repo B en el camino de A abre un proyecto propio, y su remoto se ata a él.

**Unión (`merge`).** `clawg projects merge FROM INTO [--dry-run] [--undo N]` (tras `FROM` queda lápida `merged_into = INTO`):
- repunta los localizadores de FROM a INTO;
- mueve las filas de `memory` con `merge_in` (registrado en `memory_move`, reconcilia claves y textos como R17) y las de las otras dos tablas con `project_rekey`;
- une `synthesis_review` por el máximo de `reviewed_at` (la PK `(agent_id, scope_key)` puede chocar; se anota lo que había para deshacer);
- entrega los `workspace.md` (`adopt_legacy`, `workspace.rs:57`);
- repunta `project_remote`/localizadores y `project_rekey.project_id`;
- toma un snapshot `pre-rekey` antes y escribe un diario de unión propio (localizadores, `synthesis_review`) además del `memory_move`, para que `--undo` devuelva todo.
**Lápida, no borrado.** Los ids viejos siguen vivos en exports, en `memory_move`, en `memory_import_extra`, en `project_rekey`, en snapshots y en los puentes abiertos (arriba). El daemon resuelve hacia adelante (`canonical(id)` sigue `merged_into` con un límite de saltos y detecta ciclos) en cada entrada que recibe un id: guardado, búsqueda, contexto, sesiones (`api.rs:1585`), disputas, síntesis, `PATCH`, importación. Así un puente abierto, un hook viejo o un export de otra máquina no resucitan el id absorbido.

**Detección de duplicados.** `doctor` suma hallazgos (`projects.duplicate_remote`, `.same_name`, `.same_root_commit`, `.orphan_id`) con `safe_next_step` = el `merge` en `--dry-run`. El índice único parcial impide crear dos proyectos vivos con el mismo remoto: un alta que lo repetiría se rechaza apuntando al existente.

**Alta automática y revisión** (decidido el 2026-10-24). Que exista un proyecto no puede depender de que el usuario se acuerde de correr un comando, y arreglar un duplicado tampoco:
- *Con carpeta*, el alta ya es automática: el puente registra la raíz en su primer uso (`crates/clawg-cli/src/mcp/server.rs:881`) y los hooks la del `cwd` en cada evento (`hook.rs:217-228`). Se conserva: el resolvedor decide solo con evidencia fuerte (camino validado por la huella, remoto) y, si nada coincide, abre un proyecto nuevo.
- *Sin carpeta*, el agente lo crea al guardar con un `project` que no existe. Antes de crear, el daemon busca parecidos: el nombre normalizado (minúsculas, sin separadores ni acentos, distancia de edición) y los localizadores. Si los hay, no crea: devuelve los candidatos y el agente le pregunta al usuario cuál es o si es nuevo.
- *Con evidencia débil* (el mismo primer commit con el camino viejo todavía presente, una carpeta con el nombre de un proyecto cuyo camino ya no existe, un nombre parecido a otro) se crea **separado** y queda una sugerencia pendiente («este proyecto podría ser Z»). La sugerencia sale en `session-start` y en `recall_context`, como hoy ACTIVE CONFLICTS, y el agente la pregunta y registra la respuesta: unir o mantener separados. Ante la duda, separado: unir se deshace con `--undo`, y separar lo que se fundió no se puede sin adivinar fila por fila (ver No-alcance).
- *La revisión humana es cuando el usuario quiera:* `clawg projects list` marca los proyectos creados solos y todavía no revisados; `rename` cambia el nombre y no el id; `merge X --into Z` pasa todo lo de X a Z y, para el usuario, X desaparece (no se lista, no se elige y su nombre queda libre), aunque por dentro quede la lápida que redirige; `delete` quita un proyecto vacío y, con memorias, sugiere vaciarlo en otro antes o borrarlo todo con doble chequeo (ver decisiones); `doctor` lista los duplicados sospechados con el `merge` en `--dry-run` como paso siguiente.

**Proyectos sin carpeta.** `clawg projects create NAME` (CLI, a nombre del usuario) crea identidad sin localizadores; `projects move --to-project ID|NAME` y `PATCH` los llenan; `projects locate ID --path|--remote` agrega un localizador después. Para que un agente guarde o busque en uno, `save_memory`/`search_memory`/`recall_context`/`update_memory` necesitan un argumento nuevo (`project`: id o nombre de uno existente) y, si falta o no existe, un error que lista los candidatos. Eso cambia `agent_contract.json` y `agent_contract.es.json` (re-bendecir, `CLAWG_BLESS=1`) y `tools_es.rs`. La creación es automática y pasa por la búsqueda de parecidos descrita en «Alta automática y revisión»; `clawg projects create` queda para quien quiera crear uno a mano.

**Exportación e importación.** La línea `project` gana campos opcionales (`name`, `locators` de tipo `remote` y `root_commit`, `merged_into`); los caminos son locales de la máquina y no viajan como identidad (siguen como `root_path` informativo y `--map-root`). El import resuelve por localizador (remoto, primer commit) antes que por camino, y sigue la lápida: un export hecho antes de una unión aterriza en el destino. Los proyectos que crea un import entran al diario V26 (`memory_import_extra`, nuevos `kind`). El viejo `CHECK (kind IN (...))` de V26:60 no admite tipos nuevos sin reconstruir esa tabla chica (decisión de diseño).

**Cliente anterior (`assigned: false`).** Sigue recibiendo `derive_project_id(raíz)` y el 409 de colisión (R18). Para no abrir un duplicado cuando ese id pertenece a un proyecto con otro nombre, el daemon resuelve ese id a su identidad por el localizador `path`; un id derivado que nadie conoce se registra al vuelo.

**Docs.** El índice de docs sigue por id de raíz (derivado del disco, se recalcula al cambiar sha256 o modelo, `project-docs` R*). Un proyecto sin carpeta no tiene docs; uno con varias raíces tiene el índice de cada checkout, como hoy (R16).

*Costos:* dos tablas, un resolvedor puro nuevo, una resolución hacia adelante en cada entrada del daemon, migración con relleno y su prueba a V26, un comando nuevo con deshacer, hallazgos de `doctor`, contrato MCP. *Riesgo principal:* que una entrada olvide resolver hacia adelante (se cubre con una prueba que recorra las rutas del router y con la regla de `doctor`).

### B. Mínimo: filas de proyecto sin raíz y arreglo del bug

Hacer `root_path` nullable (reconstruir `project`, una tabla chica sin vec0 ni FTS) o agregar una fila nombrada; permitir `--to-project NAME` y un `project` en `save_memory`; corregir `plan_in`.
- *Ventaja:* pocos cambios y el contrato MCP solo suma un argumento.
- *Contra:* el id de proyecto sigue siendo un id de raíz; `project.id` sigue mezclando «raíz» y «proyecto» (hoy el id de proyecto de un clone es el id de la *primera raíz*, `project.rs:191-195`), así que borrar o unir esa primera raíz sigue siendo ambiguo. Sin localizadores no hay dónde guardar la huella que valida un camino, y el arreglo del bug queda limitado a lo que se puede deducir de `project_remote`. No hay lápida: una unión borraría el id o dejaría ids sueltos.

### C. Archivo marcador en la carpeta

Un archivo con el id (por ejemplo `.git/clawg-project` o `.clawg`) que sobrevive a mudanzas y renombres de una carpeta sin remoto.
- *Ventaja:* resuelve el caso 2 sin red ni git.
- *Contra:* escribe en una carpeta del usuario (un `.clawg` entra en `git status`; dentro de `.git/` no viaja en los clones, que es lo correcto, pero no existe sin git), `cp -r` copia el marcador y deja dos carpetas con un mismo id (hay que decidir cuál conserva), un `git clean -fdx` lo borra, y complica Windows y los sistemas de solo lectura. Solo sirve como localizador opcional **con permiso** y siempre pasa por la misma validación; no como fuente de verdad.

### D. Arreglar solo el fundido, como parche aparte

Sin tablas nuevas: `root_id_in`/`plan_in` aceptan el id grabado para el camino solo si el remoto actual de la carpeta es compatible con los que `project_remote` tiene para ese proyecto; si no, toman el siguiente `candidate_id` libre (`n ≥ 1`) como proyecto nuevo.
- *Ventaja:* corrige el único defecto que corrompe aislamiento, rápido, con una prueba RED clara (mismo camino, otro repo, remoto sin registrar), compatible con los cuatro caminos (`plan_in`, `project_id_in`, `plan_ids_in`, importación).
- *Contra:* no ve el caso en que A no tenía remoto y B sí (no hay con qué compararlo sin una huella como el primer commit), no repara lo ya fundido, y como `recorded_id_in` toma la fila más vieja por camino (`project.rs:103-106`) hay que decidir cuál de dos filas con el mismo `root_path` gana (hoy `idx_project_root` no es único, V24:35). Y lo que escribe (un id nuevo para un camino ya grabado) tiene que seguir el patrón de A para no quedar como un caso especial.

### Recomendación

1. **D primero, como su propio parche** (una versión patch, o dentro del cambio con su propio commit y prueba): es un bug de integridad y no depende del rediseño. Debe incluir una regla de `doctor` que lo detecte en stores ya afectados leyendo, sin escribir, el `origin` actual de cada `root_path` contra los remotos grabados.
2. **Luego A**, en este orden de entrega: (a) tablas, relleno y resolución con huella (cierra el hueco de D con el primer commit), (b) unión con lápida y resolución hacia adelante, (c) proyectos sin carpeta y el argumento `project` del contrato, (d) export/import con localizadores.
3. **Id de los proyectos con carpeta:** conservar el actual. **Sin carpeta:** misma forma `slug-4hex`.
4. **Alta automática** (decidido): con carpeta, como hoy; sin carpeta, el agente crea por nombre después de buscar parecidos, y si los hay pregunta. La evidencia débil crea separado y deja una sugerencia pendiente que el agente pregunta. El usuario revisa, renombra, une o borra cuando quiera.
5. **C descartado como fuente de verdad**; se puede ofrecer más adelante como localizador opcional.
6. **Reglas de FK:** hallazgo de `doctor` más resolución hacia adelante y alta al vuelo; no reconstruir `memory` (ver Descartado).

## Descartado

| Opción | Razón técnica |
|---|---|
| Reescribir `memory.project_id` de las filas existentes a ids opacos nuevos | Toca 54k+ filas y su copia en `memory_fts`, rompe exports, `memory_move`, snapshots y puentes abiertos, y no resuelve nada que una tabla de identidad no resuelva sin mover una fila. Es el tipo de cambio que la regla del store del usuario pide evitar. |
| Reconstruir `memory` para agregar una clave foránea real | `memory` tiene vec0, FTS y triggers (V8); SQLite exige recrear la tabla para añadir una FK. Riesgo alto, ganancia chica frente a un hallazgo de `doctor` y el alta al vuelo; además rechazaría saves legítimos hoy (el puente escribe bajo un id sin registrar cuando falla el registro, `server.rs:885-891`) y los dejaría `failed` en el outbox. |
| Triggers que rechacen un `project_id` desconocido | Mismo problema de saves rechazados; sirve como prueba, no como puerta. |
| Unir borrando el proyecto absorbido | Los ids viejos viven en exports, diarios, snapshots y puentes abiertos; sin lápida resucitan o quedan colgando. |
| Unir automáticamente por primer commit | Un fork, una copia o un clone de un proyecto vendorizado comparten el primer commit; R16 ya fijó que «un fork es otro proyecto». Se sugiere, no se une. |
| Crear un proyecto por nombre sin buscar antes los parecidos | Produce `mi-app`, `Mi App` y `mi_app` como tres proyectos. La creación es automática, pero pasa siempre por la búsqueda de parecidos y la pregunta al usuario. |
| Exigir un comando para dar de alta un proyecto | El alta depende de que el usuario se acuerde; las carpetas ya se registran solas en su primer uso. |
| Unir solo ante evidencia débil | Separar un proyecto fundido no se puede sin adivinar fila por fila; mantener separado y preguntar sí se corrige con un `merge` reversible. |
| Reemplazar el índice de docs por uno por proyecto | Se deriva del disco y de cada checkout (`project-docs`); un proyecto sin carpeta no tiene docs. Sigue por id de raíz. |

## Interacciones

- **ranking-at-scale (86).** Las 15.709 filas sin proyecto compiten en todas las búsquedas (`search.rs:107-113`) y hacen parte del «fondo» que ese cambio mide. Colocarlas en proyectos con nombre cambia ese fondo, y por tanto cualquier línea base o umbral calibrado; hay que decidir el orden antes de medir (ver decisiones) y registrar en el informe de la medición si el fondo tenía filas sin proyecto.
- **import-job (87).** Un trabajo de importación que registre proyectos nuevos (raíces, remotos y ahora localizadores) debe pasar por el mismo resolvedor y por el diario V26; `--project-map NAME=PATH` gana un destino por nombre o id de proyecto sin carpeta (`NAME=project:ID`). Coordinar la forma del informe.
- **project-reassign (77), R8–R13 y HI3.** Mover sigue siendo «cambia el proyecto de una unidad entera y nada más». `--to-project` y `PATCH project_id` aceptan nombres y siguen la lápida. `check_target_in` (`relocate.rs:424`) pasa a consultar el registro.
- **project-clone-identity (30), R14–R19.** R14 (clave de raíz) no cambia. R15 («el registro asigna el id de raíz») y R16 («los clones de un remoto comparten el proyecto») ganan la validación por huella del camino; R16 se expresa sobre localizadores. R17 (unir reconcilia, se registra y se deshace) se reutiliza tal cual para `merge`. R18 (cliente anterior) se conserva. R19 (datos de la versión anterior) pide su equivalente para V26→V27.
- **project-identity (4, archivado).** Lo que fijó sobre `derive_project_id` y la raíz del `.git` no se contradice: la raíz sigue siendo la unidad de ubicación.
- **Hooks (`hook.rs:217-228`).** Corren en cada evento con 3 s de plazo: leer el primer commit no puede ir en el camino caliente; solo se calcula cuando el camino no está grabado o la huella contradice.
- **Sesiones y `workspace.md`.** Siguen por id de proyecto; en una unión se entregan (`adopt_legacy`) y el registro en memoria del daemon se resuelve hacia adelante.
- **Docs-search (`search_project`).** Un `project` sin carpeta devuelve el 403 de «nunca registrada con `index_docs`» o un mensaje propio.
- **Contrato del modelo.** `agent_contract.json`/`.es.json`, `protocol.md`/`.es.md` (capaz de nombrar proyectos existentes) y `tools_es.rs`.
- **sharing.json.** Sin cambio: el alcance sigue por agente, no por proyecto.

## Alcance candidato

1. Parche del fundido (D): `root_id_in`/`plan_in`/`project_id_in`/`plan_ids_in` validan el camino contra el remoto; regla de `doctor` que lo detecta en un store ya afectado.
2. V27: `project_identity` y `project_locator`, relleno que cubre todo id referenciado, `project_remote` mantenido.
3. Resolvedor puro `resolve_in` con huella (remoto y primer commit) y la regla de la mudanza (camino viejo ausente).
4. `clawg projects merge` con lápida, diario propio, `--dry-run` y `--undo`, y resolución hacia adelante en cada entrada del daemon.
5. Alta automática con búsqueda de parecidos y sugerencias pendientes en `session-start` y `recall_context`; `clawg projects create|rename|delete|locate|list` para revisar; hallazgos de duplicados en `doctor`.
6. Argumento `project` en las herramientas MCP necesarias y su contrato en las dos lenguas.
7. Export con campos opcionales de proyecto e importación por localizador y lápida; destinos sin carpeta para `--project-map`.
8. Specs: deltas de `specs/project` (R15, R16, nuevos), `specs/portability`, `specs/doctor`, `specs/mcp`, `specs/storage` (V27); `.claude/CLAUDE.md` donde cambie lo visible.

### No-alcance

- Reescribir `memory.project_id` existente, reconstruir `memory` o agregar una FK real.
- Cualquier cambio de ranking, del piso de coseno o de `memory_project_predicate!` (aunque colocar filas lo alivie).
- Archivos marcador en la carpeta del usuario (queda como decisión).
- Separar automáticamente lo que el bug ya fundió: se detecta y se ofrece mover con `projects move`; no se adivina a qué proyecto pertenece cada fila.
- Cambiar `sharing.json`, el índice de docs o el formato de `workspace.md`.
- Subir la versión del formato de export.

## Decisiones abiertas

Son del usuario.

- [ ] **Formato del id de un proyecto nuevo.** Conservar la forma `slug-4hex` (compatible con `is_project_id`, el workspace y los clientes anteriores; 16 bits, comprobados contra el registro) o un id opaco largo para los sin carpeta (más limpio, toca la validación).
- [x] **Quién crea proyectos.** Decidido el 2026-10-24: alta automática, sin depender de un comando. Antes de crear se buscan parecidos y, si los hay, se le pregunta al usuario; la evidencia débil crea separado con una sugerencia pendiente que el agente pregunta; el usuario revisa, renombra, une y borra cuando quiera (ver «Alta automática y revisión»).
- [x] **Borrar un proyecto que no es un duplicado.** Decidido el 2026-10-24, en dos etapas. (1) `delete` sobre un proyecto con memorias vivas se rechaza y sugiere vaciarlo antes, pasando todas sus memorias a otro proyecto (`merge` hacia un candidato parecido, o `projects move`); un proyecto vacío se borra sin más. (2) Si el usuario de verdad quiere sacar de la memoria todo lo del proyecto, puede borrar el proyecto con todas sus memorias, pero sólo con un doble chequeo: primero se muestra qué se borraría (por tipo, autor y estado, con las preferencias y decisiones aparte) junto con la sugerencia de vaciarlo en otro proyecto, y después hace falta una confirmación explícita de ese mismo conjunto. Lo hace el usuario por CLI, nunca un agente. Queda para la propuesta si ese borrado deja las memorias borradas y restaurables, como todo `delete`, con la purga permanente como un paso aparte, igual que hoy `delete` y `purge`.
- [ ] **Cómo responde el agente una sugerencia de proyecto.** Una herramienta nueva o un argumento en una existente (los dos cambian el contrato MCP), y si unir exige `user_answered` con la respuesta del usuario, como un veredicto sobre una decisión.
- [ ] **Vincular un camino nuevo.** Automático por remoto (como hoy) y por primer commit solo si el camino viejo ya no existe (recomendado), o siempre preguntando.
- [ ] **Leer el primer commit.** Ejecutar `git` (más simple, un proceso) o leer los objetos sin git (complejo, packfiles); y si se guarda en caché para el camino caliente de los hooks.
- [ ] **Archivo marcador.** No (recomendado); o solo opt-in y dentro de `.git/`.
- [ ] **Semántica de la unión.** Lápida (recomendado: los ids viejos siguen resolviendo en exports, diarios, snapshots y puentes abiertos) o borrado (menos filas, pero rompe todo eso). Para el usuario, X desaparece en los dos casos; la diferencia es lo que queda por dentro.
- [ ] **Clave foránea.** Hallazgo de `doctor` más alta al vuelo (recomendado) o reconstruir `memory` con FK real (ver Descartado).
- [ ] **El parche del fundido va primero.** Recomendado: sí, solo; si no, dentro de este cambio como primer commit.
- [ ] **Las 15.709 filas sin proyecto hasta que esto salga.** Esperar, o colocarlas ya en carpetas provisionales con `projects move --to PATH` (reversible con `--undo N`, pero deja caminos inventados en `project` y proyectos que luego hay que unir). Y si se colocan antes de medir ranking-at-scale.
- [ ] **Reimportar tras mover.** Aceptar que un export viejo inserta de nuevo bajo su proyecto de origen (lo correcto para otro store), arreglando solo el caso de la unión con lápida; o buscar presencia en cualquier proyecto por `(agente, hash, created_at)`, con el riesgo de colapsar un mismo texto legítimo en dos proyectos.
- [ ] **`project_remote`.** Mantenerla como tabla o reemplazarla por el localizador `remote` con una vista.
- [ ] **Qué hace `search_memory` sin `project` ni `project_path`** cuando el cwd no tiene proyecto resoluble: igual que hoy (sin alcance) o error.

## Cómo se mide

**Pruebas RED (fallan hoy):**
1. `project.rs`: se registran el repo A (remoto `acme/a`) en `P` y luego el repo B (remoto `acme/b`, nunca registrado) en el mismo `P`; hoy las dos raíces dan el mismo `project_id` y `project_remote[acme/b]` apunta a A; con el arreglo, B tiene proyecto propio y su remoto apunta a él. Un tercer clone de `acme/b` en `Q` se une a B, no a A.
2. Lo mismo sin `assigned`: hoy no hay 409 para el mismo camino; con el arreglo el cliente anterior sigue sin unirse por remoto y no se le asigna el proyecto ajeno (o recibe el 409 que se decida).
3. Mudanza: repo local sin `origin`, mismo primer commit, camino viejo ausente → el nuevo camino se vincula al proyecto; con el camino viejo presente → proyecto nuevo y sugerencia.
4. Unión: `merge A B` con filas en las tres tablas, `synthesis_review` en ambos y un `workspace.md` en ambos; verificar que todo queda bajo B, que `--undo N` deja el store como estaba (filas, estados, localizadores, `synthesis_review`, archivos) y que un guardado con el id A hecho después se resuelve a B.
5. Un export hecho antes de la unión se importa en el destino sin crear el proyecto absorbido.
6. Migración (equipo de pruebas obligatorio por `openspec/config.yaml`): sembrar un store en V26 con dos raíces que comparten remoto, una con colisión de hash, filas en `memory`/`episodic_trajectory`/`procedural_skill`, `memory_move`, `project_rekey`, `synthesis_review` y un `project_id` sin fila en `project`; abrir con V27 y comprobar que ninguna fila cambió, que cada id tiene identidad, que `search`/`list`/`context` devuelven lo mismo que antes y que `doctor` no marca nada nuevo. Rollback: el snapshot `pre-migration` que ya escribe la apertura (`.claude/CLAUDE.md`, Data profiles) y la negativa de un binario anterior a abrir un store más nuevo.
7. Un recorrido de todas las rutas del router que reciben un id de proyecto: cada una resuelve hacia adelante (falla si se agrega una ruta sin hacerlo).
8. Contrato: `agent_contract.json` y `.es.json` con el argumento `project`; un proyecto inexistente da un error que lista candidatos.
9. Fusión de datos reales, medición manual con un store de desarrollo: el hallazgo de `doctor` del paso 1 sobre una copia del store con un caso fundido.
10. Alta sin carpeta: un `project` que no existe y sin parecidos crea el proyecto; con un parecido (`Mi App` frente a `mi-app`) no crea y devuelve los candidatos.
11. Evidencia débil: una carpeta nueva con el nombre de un proyecto cuyo camino ya no existe abre un proyecto separado y deja una sugerencia que `session-start` y `recall_context` muestran hasta que se responde; responder «separados» la quita para siempre.
12. Revisión: `rename` cambia el nombre, conserva el id y todo lo guardado sigue alcanzable; después de `merge X --into Z`, X no aparece en `projects list` ni se puede elegir.

**Aceptación:** ninguna fila de `memory` cambia de `project_id` por la migración; todo lo guardado sigue alcanzable; cada operación de unión es reversible; ningún texto, camino o nombre de persona o de proyecto de un cliente entra en pruebas ni en documentos versionados.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| Una entrada del daemon olvida resolver la lápida y un puente abierto sigue escribiendo bajo el id absorbido | Función única `canonical(id)`; prueba que recorre el router; regla de `doctor` sobre filas bajo ids con `merged_into` |
| Falsos positivos de la huella (un remoto que cambia de nombre, un repo que se mueve de organización) parten un proyecto en dos | La huella solo contradice si hay dos valores *conocidos* y distintos; la unión explícita y `doctor` lo corrigen; nunca se borra nada |
| Falsos negativos (A sin remoto y B con remoto en el mismo camino sin primer commit legible) siguen fundiendo | Aviso en `doctor` (carpeta sin huella con memorias de otro `origin`); el primer commit lo cubre para git |
| El relleno deja un id referenciado sin identidad | El relleno une todas las tablas con id; la prueba de migración compara conteos; el alta al vuelo cubre lo posterior |
| Dos proyectos con nombre parecido creados por agentes | Búsqueda de parecidos antes de crear y pregunta al usuario; sugerencias pendientes; el índice único de remoto; hallazgo de duplicados en `doctor`; unir es reversible |
| Preguntar de más: una sugerencia que vuelve en cada sesión cansa | Cada sugerencia se pregunta hasta que se responde; la respuesta «separados» queda guardada y ese par no vuelve a salir |
| Deshacer una unión pierde lo guardado después | `undo_move_core` ya mueve solo lo que sigue donde la unión lo dejó; el diario propio hace lo mismo con localizadores y `synthesis_review` |
| Lectura del primer commit en el camino caliente de los hooks (plazo de 3 s) | Solo cuando el camino no está grabado o la huella contradice; sin git, sin esa huella |
| El contrato MCP cambia el prompt del modelo | Re-bendecir los dos contratos y revisar el diff; el argumento es opcional y su ausencia reproduce el comportamiento actual |
| Un store V27 abierto por un binario anterior | Un binario anterior rechaza un store más nuevo sin tocarlo; el rollback es el snapshot `pre-migration` (`update` ya lo restaura en proceso) |
| El tamaño del cambio (M–L) | Entrega en cuatro pasos con prueba propia (parche, tablas y resolución, unión, sin carpeta y export), cada uno con sus commits atómicos |
