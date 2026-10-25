# project-locators — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 | M–L, en cuatro entregas M | project-registry (89), archivado: su entrega 0 sale en 0.63.0 (R20 de [`specs/project`](../../specs/project/spec.md)). Se construye sobre R1–R20, de project-reassign (77) y project-clone-identity (30) | ver [`state.yaml`](state.yaml) |

Este cambio nació al partir [project-registry](../archive/2026-10-25-project-registry/proposal.md) (89) cuando se archivó. La entrega 0, el arreglo del fundido, quedó allí. Las entregas 1 a 4 y las decisiones que las ordenan pasaron aquí, con la numeración de entonces. La [exploración](../archive/2026-10-25-project-registry/exploration.md) sigue siendo la de 89: el mapa de dónde se deriva, se guarda y se consume un id de proyecto, los enfoques A a D y lo descartado. Las citas de esta propuesta apuntan al código de después de la entrega 0.

## Problema

Hoy un proyecto *es* una carpeta. Su id sale del camino (`derive_project_id`, `crates/clawg-core/src/memory/identity.rs:11-15`) y `project.root_path` es `NOT NULL` (`crates/clawg-core/migrations/V9__project.sql:6-10`). Desde 0.63.0 un camino puede tener varias filas, una por repositorio, y la fila sirve a una raíz sólo si su remoto no la contradice (R20; `settle`, `crates/clawg-core/src/memory/project.rs:126-136`). Quedan cuatro fallas, de la más grave a la menos grave:

1. **Los dos límites de R20.** Los dos salen de lo mismo: sin el primer commit, un camino no tiene otra huella que su remoto.
   - **Un repositorio en el camino de otro sigue fundiéndose cuando falta un remoto.** Una clave que no se puede leer no contradice nada, así que un repositorio sin `origin` se queda con la fila más vieja del camino (`settle`, `project.rs:130-132`). Además, una fila sin remotos atados acepta como primer remoto la clave del que llega (`:135`). Si al anterior o al recién llegado le falta `origin`, el recién llegado se queda con la fila del anterior y con sus memorias.
   - **Un remoto renombrado abre otro proyecto sin decir nada.** Si cambia el `origin` de una raíz ya registrada (el repositorio se renombró o cambió de organización), el camino queda contradicho y la raíz recibe una fila nueva (`settle_root_in`, `project.rs:188-208`). Sus memorias quedan bajo el id viejo, y sólo `doctor` (`project_remotes`) lo nombra. R16 perdió así la garantía de que un remoto nuevo conservaba el proyecto.
2. **Una carpeta sin `origin` (o sin VCS) que se mueve o se renombra pierde su proyecto.** El camino nuevo no está grabado y recibe el primer candidato libre (`project.rs:201-206`). `legacy_ids` sólo cubre el camino por el que se llegó a la misma raíz (`identity.rs:54-58`). `doctor` no lo ve, porque el id viejo sigue siendo un candidato de su raíz vieja (`crates/clawg-cli/src/doctor/checks.rs:561-585`).
3. **No existe un proyecto sin carpeta.**
   - `--to-project` exige una fila de `project` (`check_target_in`, `crates/clawg-core/src/memory/relocate.rs:426-436`), y esa fila sólo nace de una raíz.
   - Un `project_path` inventado se registra como una raíz normalizada a mano (`identity.rs:67-74`), así que su identidad es el hash de un camino que no existe.
   - Un subproyecto de un monorepo tampoco puede ser un proyecto propio: la raíz sube hasta el primer `.git` (R1 de [`specs/project`](../../specs/project/spec.md)).
   - Por eso no se pueden colocar sin inventar carpetas las 15.709 filas que el import dejó sin proyecto en el store de producción. La cifra se midió allí y no se releyó en este repo. Mientras tanto, esas filas compiten en la búsqueda de cada proyecto (`memory_project_predicate!`, `crates/clawg-core/src/memory/search.rs:107-113`).
4. **No hay dónde unir, renombrar o borrar un proyecto.**
   - `merge_in` (`relocate.rs:884`) sólo lo dispara el registro (`project.rs:356`).
   - Un duplicado se arregla a mano con `projects move` por filtros. Eso no lleva `synthesis_review`, `workspace.md`, `episodic_trajectory` ni `procedural_skill`.
   - Nada garantiza que cada `memory.project_id` esté en `project`. El puente guarda los ids de cada raíz durante toda la sesión (`crates/clawg-cli/src/mcp/server.rs:874-878`). Si el registro falla, escribe bajo el id derivado sin registrarlo (`:881-892`).

## Objetivo

Objetivo 2: separar las memorias por proyecto y, a la vez, poder usarlas entre proyectos. Para separar hace falta una identidad que no se confunda con la carpeta donde vive un proyecto en un momento dado. Además, que un proyecto quede «sin proyecto» por accidente tiene que dejar de ser la única forma de usarlo en todos lados. Lo global no cambia: una fila sin proyecto y una `user_preference` se siguen leyendo desde cualquier proyecto (`search.rs:107-113`), y `all_projects` sigue buscando en todos.

## Decisiones

Se tomaron el 2026-10-24 sobre las [decisiones abiertas](../archive/2026-10-25-project-registry/exploration.md#decisiones-abiertas) de la exploración de 89, y se conservan sus números:

1. **La identidad se separa de la ubicación (enfoque A).**
   - `project` pasa a ser la tabla de referencia de los proyectos: id estable, nombre y lápida de unión. Todas las tablas guardan su id.
   - Las formas de llegar a un proyecto, sus **localizadores**, son opcionales, de cero a muchas: sus carpetas (`project_path`, que es la tabla `project` de hoy renombrada, con las huellas de cada una) y sus remotos (`project_remote`). El primer commit es una huella de la carpeta.
   - Una carpeta sólo vincula si lo valida una huella que no depende del camino.
   - Un proyecto puede no tener carpeta (una idea), haberla perdido (se borró o se movió), tener varias (clones, mudanzas) o ser una carpeta dentro de un monorepo.
   - Los ids que ya existen (`nombre-1a2b`) se conservan y **ninguna fila de `memory` cambia de `project_id`**. V27 renombra la tabla de carpetas sin cambiar ningún valor, agrega lo nuevo y lo rellena desde las tablas que guardan un id de proyecto.
   - El modelo se ajustó así el 2026-10-25, al escribir el [diseño](design.md#el-modelo) de la entrega 1.
2. **El arreglo del fundido iba primero y solo.** Se cumplió: es la entrega 0 de 89 (R20), que sale en 0.63.0. Su límite conocido lo cierra la huella del primer commit en la entrega 1.
3. **Alta automática.** Nada puede depender de que el usuario se acuerde de correr un comando.
   - Las carpetas se siguen registrando solas en su primer uso (`server.rs:881`, `crates/clawg-cli/src/hook.rs:220-228`).
   - Un proyecto sin carpeta lo crea el agente cuando guarda con un nombre de `project` que no existe. Antes, el daemon busca parecidos (nombre normalizado, localizadores). Si los hay, no crea nada: devuelve los candidatos para que el agente le pregunte al usuario.
   - Ante evidencia débil se crea un proyecto **separado** y una sugerencia pendiente. Evidencia débil es el mismo primer commit con el camino viejo todavía presente, una carpeta con el nombre de un proyecto cuyo camino ya no existe o un nombre parecido. `session-start` y `recall_context` muestran la sugerencia, como hoy ACTIVE CONFLICTS, hasta que el agente la pregunta. Una respuesta «separados» se guarda y no se vuelve a preguntar.
   - Ante la duda, separado: una unión se deshace, y lo fundido no se puede separar.
4. **El usuario revisa cuando quiere.**
   - `clawg projects list` marca los proyectos que se crearon solos y todavía no se revisaron.
   - `rename` cambia el nombre y nunca el id.
   - `merge X --into Z` pasa todo lo de X a Z. Para el usuario, X desaparece: no se lista, no se elige y su nombre queda libre. Por dentro queda una lápida `merged_into` que redirige, porque los puentes abiertos guardan los ids durante toda la sesión (`server.rs:874-878`) y los exports viejos llevan X.
   - `doctor` lista los duplicados sospechados y propone como paso siguiente el `merge` en `--dry-run`.
5. **Borrar un proyecto: dos etapas.**
   - **Etapa 1.** Si el proyecto tiene memorias vivas, `delete` se rechaza y propone vaciarlo antes: un `merge` hacia un candidato parecido, o `projects move`. Un proyecto vacío se borra. Vacío quiere decir sin filas no borradas en las tres tablas con proyecto.
   - **Etapa 2.** Sacar de la memoria todo lo de un proyecto pide un doble chequeo, por CLI y nunca por un agente. Primero, un informe de lo que se borraría por tipo, autor y estado, con las `user_preference` y `architectural_decision` aparte y la sugerencia de vaciarlo en otro proyecto. Después, la confirmación explícita de ese mismo conjunto (la elección está más abajo).
6. **Forma del id de un proyecto nuevo**: `slug-4hex`, compatible con `is_project_id` (`project.rs:84`). Se comprueba libre contra el registro, como hace el bucle de candidatos (`project.rs:201-206`).
7. **Vincular un camino nuevo**: es automático por remoto, como hoy. Por primer commit, sólo cuando el camino viejo ya no existe (una mudanza). En cualquier otro caso queda una sugerencia.
8. **Las 15.709 filas sin proyecto esperan a este cambio**, sin carpetas provisionales. Cuando exista un proyecto sin carpeta (entrega 3), se colocan con `clawg projects move --origin-project NOMBRE --to-project PROYECTO`.

### La elección para la etapa 2

**Las filas quedan borradas y se pueden restaurar, como con cualquier `delete`.** La etapa 2 tiene un número de borrado y su `--undo N`. La eliminación permanente sigue siendo un paso aparte, con `clawg purge`. Hoy `purge` toma un solo id (`crates/clawg-core/src/memory/secrets.rs:181`); gana `--project-delete N`, que borra para siempre las filas que dejó borradas el borrado N. Lo hace con las mismas reglas (daemon parado, sin snapshot) y con los mismos avisos de snapshots y de `workspace.md` que todavía guardan el texto (`secrets.rs:129-131`).

- **Cómo prueba el segundo paso que el conjunto es el mismo.**
  - El primer paso (`clawg projects delete X --with-memories`) no escribe nada. Imprime el informe y un código corto, que resume lo que se borraría: el id del proyecto, sus localizadores y, por cada fila de las tres tablas, su id, `status`, `deleted_at` y `update_count`.
  - El segundo paso (`… --confirm CÓDIGO`) recalcula el código dentro de la transacción que escribe. Si llegó un save, se editó una fila o cambió un localizador, se rechaza sin escribir y muestra el informe nuevo.
  - No se guarda estado: el código vence solo en cuanto cambia el conjunto.
- **Lo que tiene en cuenta:**
  - **Revivir.** Borrar la fila vigente de una clave revive la anterior (`plan_revival`, `crates/clawg-core/src/memory/revival.rs:42-55`; `delete_memory_core` lo corre en cada borrado, `crates/clawg-core/src/memory/admin.rs:449-453`). Las filas del proyecto se marcan juntas, y `REPLACED_SQL` sólo mira filas no borradas (`revival.rs:73`), así que **no revive nada dentro del proyecto**. Una fila de afuera que un veredicto reemplazó sigue reemplazada, como con cualquier borrado.
  - **Filas de otros agentes.** Una llamada del usuario alcanza todas las filas, y el informe las cuenta por autor.
  - **Saves en el outbox.** El evento lleva el proyecto (`crates/clawg-core/src/memory/events.rs:15`), así que un save pendiente para el proyecto llegaría después del borrado. Por eso la etapa 2 se rechaza mientras haya uno, y nombra los `failed` con su `clawg outbox discard`.
  - **Pares de conflicto que cruzan a otro proyecto.** Quedan como están, y se cuentan.
  - **Conclusiones de afuera derivadas de sus filas.** Pasan a `to_review`, y el informe las cuenta.
  - **Los `workspace.md` del proyecto** (`crates/clawg-core/src/memory/workspace.rs:44`). Se nombran y no se tocan.
  - **El índice de docs de sus raíces.** Se deriva del disco: se quita, se baja `index_docs` y se desvinculan los localizadores. El `--undo` los vuelve a vincular, y el próximo refresco reconstruye el índice.
  - **Los snapshots que guardan el texto.** Los nombra `purge`, como hoy.
  - **Un snapshot antes de escribir**, como `pre-move` y `pre-import`. El `--undo` necesita el modelo: un borrado quita los vectores (`admin.rs:441`) y restaurar los recalcula, como `undelete`.
- **La alternativa: una purga permanente del proyecto en un paso.**
  - A favor: un solo comando para quien de verdad quiere que no quede nada.
  - En contra: sólo se deshace con un snapshot, que guarda justo el texto que se quería sacar. Exige el daemon parado y reconstruir el archivo, como `purge`. Y junta en un mismo paso el doble chequeo de la etapa 2 con lo irreversible.
  - Con los pasos separados se mantiene la regla de hoy: un `delete` se deshace, y `purge` es lo único que no tiene vuelta.

## Entregas

Cada entrega se publica sola, con commits atómicos y TDD estricto. Los tests RED salen de [«Cómo se mide»](../archive/2026-10-25-project-registry/exploration.md#cómo-se-mide) de la exploración, más los que piden las decisiones de arriba. La spec, el diseño y las tareas se escriben por entrega, cuando le toca a cada una.

### 1. El registro (V27)

**Qué trae.**
- **V27**:
  - `project` pasa a ser la referencia: id, nombre, cómo nació y si se revisó;
  - la tabla de carpetas de hoy pasa a `project_path`, con el proyecto al que lleva y sus huellas (remoto y primer commit);
  - `project_suggestion` guarda el par, el motivo y la respuesta.

  El detalle está en el [diseño](design.md) y en [Migración](#datos-guardados-migración-y-vuelta-atrás). `merged_into` llega con la entrega 2.
- **El resolvedor puro `resolve_in(ubicación, huellas, registro)`**: primero el camino validado, después el remoto, después el primer commit. El primer commit vincula sólo si el camino viejo ya no existe; si no, abre un proyecto separado con una sugerencia. Si nada coincide, abre un proyecto nuevo. Absorbe a `settle` (R20), que hoy decide con el remoto solo.
- **La lectura del primer commit, fuera del camino caliente**: sólo cuando el camino no está grabado o la huella lo contradice.
- **`clawg projects list`**, que marca lo que se creó solo sin revisar y las sugerencias pendientes, y **`clawg projects rename`**.
- **Los clientes no notan nada**: reciben los ids de siempre.

**Tests RED.**
- Un repo local sin `origin`, con el mismo primer commit y el camino viejo ausente, se vincula a su proyecto. Con el camino viejo presente, abre un proyecto nuevo y deja una sugerencia.
- A sin remoto en `P` y después B con remoto en `P`, con otro primer commit, dan dos proyectos. Lo mismo con A y B sin remoto. Esto cierra el primer límite de R20.
- Un remoto renombrado en el mismo camino, con el mismo primer commit, abre otro proyecto y deja una sugerencia de unión, en lugar de una separación muda. Esto cierra el segundo límite de R20.
- Un camino grabado que no se contradice no lee el primer commit.
- `rename` conserva el id, y todo lo guardado sigue alcanzable.
- `projects list` marca lo que se creó solo y la sugerencia.
- El test de la base en V26 (ver [Migración](#datos-guardados-migración-y-vuelta-atrás)).

### 2. Unir, revisar y borrar

**Qué trae.**
- **`clawg projects merge X --into Z [--dry-run] [--undo N]`**:
  - las filas de `memory` pasan con `merge_in` (R17), y las otras dos tablas con `project_rekey`;
  - `synthesis_review` toma el máximo de `reviewed_at`, porque su clave primaria `(agent_id, scope_key)` puede chocar (`V10__synthesis.sql:22-27`);
  - los `workspace.md` pasan con `adopt_legacy` (`workspace.rs:57`);
  - los localizadores se reapuntan y X queda con la lápida;
  - antes se toma un snapshot `pre-rekey`, y el `--undo` tiene su propio diario.
- **`canonical(id)`**, que sigue `merged_into` con un tope de saltos y detecta ciclos. Corre en cada entrada del daemon que recibe un id de proyecto: guardar, buscar, contexto, sesiones, disputas, síntesis, `PATCH`, mover e importar.
- **Las respuestas a las sugerencias por CLI**: unir o mantener separados.
- **Los hallazgos de duplicados en `doctor`**: el mismo nombre, el mismo primer commit o un id sin identidad.
- **`delete` en sus dos etapas**, con su `--undo N`, y **`clawg purge --project-delete N`**.
- **Una migración aditiva propia** para los diarios de unión y de borrado, con su test sembrado en V27.

**Tests RED.**
- `merge A --into B`, con filas en las tres tablas y con `synthesis_review` y un `workspace.md` en los dos: todo queda bajo B. `--undo N` deja el store como estaba: filas, estados, localizadores, `synthesis_review` y archivos.
- Después de la unión, un save con el id A cae en B. A no sale en `projects list`, no se elige por nombre y su nombre queda libre.
- Un recorrido de las rutas del router que reciben un id de proyecto falla si se agrega una ruta que no sigue la lápida.
- Etapa 1: el `delete` de un proyecto con memorias vivas se rechaza, las cuenta y propone el `merge` hacia el candidato parecido. Uno vacío se borra.
- Etapa 2, primer paso: no escribe nada y da un código. Un save o una edición posteriores invalidan ese código.
- Etapa 2 con el código correcto: borra todo, no revive nada dentro del proyecto y deja como estaban las filas de afuera que un veredicto reemplazó. Se rechaza mientras haya un save pendiente para el proyecto.
- Etapa 2, `--undo N`: restaura las filas (las vuelve a embeber) y sus localizadores.
- `purge --project-delete N` borra para siempre y nombra los snapshots y los `workspace.md` que todavía guardan el texto.

### 3. Proyectos sin carpeta y el agente

**Qué trae.**
- **Un argumento opcional `project`** (id o nombre) en las herramientas que hoy toman `project_path` y en `update_memory`. Un nombre que no existe y no tiene parecidos crea el proyecto antes de escribir el outbox. Uno con parecidos no crea nada y responde los candidatos, como un 400 que no escribe nada.
- **Las sugerencias pendientes en `session-start` y en `recall_context`**, y el camino para que el agente las responda.
- **Nombres donde hoy van ids**: `--to-project` y `PATCH project_id` aceptan nombres.
- **`clawg projects create|locate`**, para quien quiera crear o ubicar un proyecto a mano.
- **Los subproyectos de un monorepo** ([diseño, D14](design.md#d14-lo-que-esta-entrega-deja-preparado-para-las-carpetas-opcionales)):
  - una carpeta declarada dentro de un repositorio;
  - la resolución elige la carpeta registrada más profunda que contiene el camino, para lo que el cliente manda también el camino crudo;
  - en otra máquina, el remoto más la subcarpeta, lo que cambia la clave de `project_remote`.
- **Una carpeta sin VCS que se mueve**: la sugerencia por nombre de la decisión 3, o `projects locate`.
- **El contrato del modelo**: `agent_contract.json` y `.es.json` vueltos a bendecir, `protocol.md`, `protocol.es.md` y `tools_es.rs`.
- **Después de esta entrega, un paso operativo** que no es código: colocar las filas sin proyecto de producción.

**Tests RED.**
- Un `project` inexistente y sin parecidos crea el proyecto. Con un parecido (`Mi App` frente a `mi-app`) no crea nada y devuelve los candidatos.
- Una carpeta nueva con el nombre de un proyecto cuyo camino ya no existe abre un proyecto separado y deja una sugerencia. `session-start` y `recall_context` la muestran hasta que se responde, y responder «separados» la quita para siempre.
- Los dos contratos cambian sólo por el argumento, y sin él el comportamiento es el de hoy.
- `search_project` sobre un proyecto sin carpeta responde su propio mensaje.
- Un subproyecto declarado en `monorepo/apps/web` recibe lo que se guarda desde `monorepo/apps/web/src`. Lo que se guarda desde `monorepo/` sigue en el proyecto del monorepo, y un clone del monorepo en otra máquina llega a los dos.

### 4. Exportar e importar

**Qué trae.**
- **La línea `project` del formato v1 gana campos opcionales**: `name`, los localizadores `remote` y `root_commit`, y `merged_into`. Los caminos no viajan como identidad: siguen como `root_path` informativo y como `--map-root`.
- **El import resuelve por localizador antes que por camino y sigue la lápida.** `--project-map NOMBRE=project:ID|NOMBRE` acepta un destino sin carpeta.
- **Lo que registra un import entra al diario V26.** El `CHECK` de `memory_import_extra` (`V26__import_journal.sql:60`) no admite tipos nuevos sin reconstruir esa tabla chica; se resuelve en el diseño.

**Tests RED.**
- Un export hecho antes de una unión se importa en el destino sin crear el proyecto absorbido.
- Un export de otra máquina con el mismo remoto cae en el proyecto local.
- Un proyecto sin carpeta va y vuelve por nombre.

## Alcance

1. V27 y su relleno, el resolvedor con huella, la lectura del primer commit, las sugerencias y `projects list|rename`.
2. `merge` con lápida, diario y `--undo`, y la resolución hacia adelante. `delete` en dos etapas con `--undo`, y `purge --project-delete`. Los hallazgos de `doctor`.
3. Los proyectos sin carpeta: el alta automática con búsqueda de parecidos, el argumento `project` del contrato en las dos lenguas, `projects create|locate` y los nombres en `--to-project` y `PATCH`.
4. El export con campos opcionales y el import por localizador y lápida.
5. Las specs: deltas de `project` (R15, R16, R20 y requisitos nuevos), `storage`, `doctor`, `mcp`, `hooks`, `portability`, `secrets` y `restore` (por `purge`). También `.claude/CLAUDE.md` donde cambie lo visible.

## Fuera de alcance

- Reescribir `memory.project_id`, reconstruir `memory` o agregarle una clave foránea real: tiene vec0, FTS y triggers (V8), y una FK rechazaría saves que hoy se aceptan (`server.rs:881-892`).
- Separar automáticamente lo que el bug ya fundió. Se detecta (`doctor`, `project_remotes`) y se ofrece `projects move`, pero no se adivina fila por fila.
- Cambios de ranking, del piso de coseno o de `memory_project_predicate!`.
- Un archivo marcador en la carpeta del usuario como fuente de verdad.
- Cambiar `sharing.json`, el índice de docs (sigue por id de raíz) o el formato de `workspace.md`.
- Subir la versión del formato de export.
- Unir automáticamente por primer commit: un fork y una copia lo comparten, y R16 fija que un fork es otro proyecto.

## Datos guardados: migración y vuelta atrás

- **Entrega 1: V27.**
  - **Antes de migrar**, la apertura escribe el snapshot `pre-migration` (`crates/clawg-core/src/memory/db.rs:237`).
  - **El renombre.** La tabla `project` de hoy pasa a `project_path` sin cambiar ningún valor, y `project` se crea como la referencia ([diseño, D1](design.md#d1-project-es-la-referencia-y-las-carpetas-son-opcionales)).
  - **El relleno** crea un proyecto por cada id que tiene filas en `memory`, `episodic_trajectory` o `procedural_skill`, o un remoto en `project_remote`. El diseño deja afuera los ids que sólo nombran los registros de historia, y las carpetas de clones donde nunca se guardó nada: serían proyectos vacíos ([D6](design.md#d6-la-referencia-y-su-relleno)). Si alguno vuelve a tener filas, lo recibe en esa escritura ([D7](design.md#d7-cada-id-que-se-escribe-se-adopta-con-un-disparador)).
  - **El nombre** es la parte anterior al último `-`.
  - **Las carpetas.** Cada una apunta a su propio id cuando ese id es un proyecto. Donde el store no puede saber a qué proyecto lleva (la carpeta de un clone), queda sin apuntar, y su primer registro lo graba. Desde 0.63.0 un camino puede tener varias carpetas (R20), y por eso el camino no es una clave única.
  - **Lo que crea el relleno no se marca «sin revisar».**
  - **Ninguna fila de `memory` cambia.**
- **El test sembrado en V26.**
  - El store tiene:
    - dos raíces que comparten remoto y una colisión de hash;
    - filas en las tres tablas;
    - entradas en `memory_move`, `project_rekey` y `synthesis_review`;
    - un `project_id` sin fila en `project`;
    - un caso fundido (dos remotos atados a un proyecto);
    - un camino con dos filas (R20).
  - Se abre con V27 y se comprueba:
    - que ninguna fila cambió y que las carpetas están en `project_path`;
    - que cada id en uso tiene su proyecto;
    - que `list` devuelve lo mismo que antes;
    - que `doctor` no marca nada nuevo fuera del caso fundido.
- **Las entregas 2 a 4** traen cada una su migración aditiva, con su test sembrado en la versión anterior.
- **Volver atrás del esquema.**
  - Un binario anterior rechaza un store más nuevo sin tocarlo (`SchemaNewer`, `db.rs:443`).
  - `clawg update` restaura en el proceso el snapshot `pre-migration` cuando el binario viejo rechaza el store (`.claude/CLAUDE.md`, `clawg update`). A mano, se hace con `clawg restore` de ese snapshot.
  - El diseño deja también el SQL que deshace V27 a mano, conservando lo guardado después, como hizo [project-reassign](../archive/2026-10-22-project-reassign/proposal.md#datos-guardados) ([diseño, «Volver atrás»](design.md#volver-atrás)).
- **Volver atrás de una operación.** Una unión, con `projects merge --undo N` o su `pre-rekey`. Un borrado de etapa 2, con `projects delete --undo N` o su snapshot. Lo que se purgó, sólo con un snapshot.

## Compatibilidad

- **Clientes sin `assigned`.** Conservan R18 y el 409 de R20. Un id derivado que nadie conoce se registra al vuelo.
- **Puentes abiertos.** Guardan los ids durante toda la sesión (`server.rs:874-878`), y `canonical(id)` los lleva al destino de una unión.
- **Hooks.** Tienen 3 s por evento: el primer commit nunca se lee para un camino grabado y sin contradicción.
- **Export v1.** Los campos nuevos son opcionales y un lector ignora los que no conoce, así que la versión del formato no cambia.
- **Contrato MCP.** El argumento es opcional y, sin él, el comportamiento es el de hoy. Se vuelven a bendecir los dos contratos, y la apertura de cada variante de `protocol` sigue en 2048 caracteres o menos.
- **`doctor`, chequeo `projects`.** Compara cada carpeta con los candidatos de su raíz (`checks.rs:561-585`), leyéndola de `project_path` desde V27. Un proyecto sin carpeta no tiene ninguna que comparar, y la segunda carpeta de un camino es un candidato de su raíz, así que no se marca ninguno de los dos. `project_remotes` sigue leyendo `project_remote`, que se conserva ([diseño, D2](design.md#d2-project_remote-sigue-siendo-una-tabla)).

## Interacciones

- **project-registry (89), archivado.** R20 es la base. El resolvedor de la entrega 1 absorbe `settle` y agrega el primer commit como segunda huella. Los tests de R20 tienen que seguir pasando. `a_different_remote_in_a_registered_path_is_another_project` sigue dando otro proyecto, y con el mismo primer commit también una sugerencia (decisiones 3 y 7).
- **ranking-at-scale (86).** Las filas sin proyecto forman parte del fondo que mide, y colocarlas lo cambia. Este cambio va antes ([README](../../README.md)): se colocan antes de su línea base, y su informe dice si el fondo tenía filas sin proyecto.
- **import-job (87).** Un trabajo de importación que registre proyectos pasa por el mismo resolvedor y por el diario V26. `--project-map` gana el destino `project:ID|NOMBRE`. La forma del informe se coordina.
- **project-reassign (77).** Mover sigue siendo «cambia el proyecto de una unidad entera y nada más» (R8). `check_target_in` consulta el registro y sigue la lápida.
- **project-clone-identity (30).**
  - R14 no cambia, y R17 se reutiliza para `merge`.
  - R15 y R16 ya validan el camino por el remoto (R20). Con el primer commit, el remoto renombrado de R16 sigue abriendo otro proyecto, pero ahora deja una sugerencia de unión.
  - R18 se conserva. R19 pide su equivalente de V26 a V27.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| Hasta la entrega 1, un repo cuyo `origin` cambia (se renombró o cambió de organización) abre un proyecto nuevo, y sus memorias quedan bajo el id viejo | Nada se borra: siguen alcanzables con `all_projects`, y `projects move --from-project VIEJO --to-project NUEVO` las une (se deshace). La regla de `doctor` `project_remotes` nombra los dos remotos. La entrega 1 le suma una sugerencia de unión |
| Una entrada del daemon olvida resolver la lápida | Una sola `canonical(id)`, un test que recorre el router y la regla de `doctor` sobre filas bajo un id con `merged_into` |
| El relleno deja un id sin identidad | El relleno recorre todas las tablas con id, el test de migración compara conteos y el alta al vuelo cubre lo que llegue después |
| El relleno ata un camino con dos filas (R20) a una sola identidad y repite el fundido | El test sembrado lleva un camino con dos filas, y cada una conserva su proyecto |
| Los agentes crean duplicados con nombres parecidos | Búsqueda de parecidos antes de crear, sugerencias pendientes, índice único por remoto, `doctor` y un `merge` reversible |
| Una sugerencia que vuelve en cada sesión cansa | Se pregunta hasta que se responde, y «separados» queda guardado |
| La etapa 2 borra más de lo que el usuario vio | El código resume el conjunto exacto y se recalcula en la transacción que escribe. Las filas quedan restaurables hasta `purge` |
| Un `--undo` de borrado choca con un proyecto que tomó el camino mientras tanto | El índice único parcial lo detecta: el `--undo` restaura las filas y deja el camino donde está, con una sugerencia de unión |
| Leer el primer commit cuesta (historia larga, varios commits raíz, clone superficial, sin `git`) | Sólo se lee fuera del camino caliente, y sin huella si no se puede leer. El diseño elige cómo leerlo |
| El contrato del modelo cambia | Los dos contratos se vuelven a bendecir, se revisa el diff y el argumento es opcional |
| El tamaño (M–L) | Cuatro entregas publicables por separado, cada una con sus tests y commits atómicos |

## Abierto para el diseño

El [diseño de la entrega 1](design.md) resolvió:

- el localizador `path`: la carpeta con sus huellas, en `project_path` (D1);
- `project_remote`: se conserva (D2);
- cómo leer el primer commit (D3);
- la integridad referencial (D7);
- el nombre (D8).

Lo demás espera a su entrega ([diseño, «Lo que espera a otra entrega»](design.md#lo-que-espera-a-otra-entrega)). La lista queda como se escribió:

- **El localizador `path` frente a R20.** La propuesta pedía un índice único parcial sobre `(kind, value)` mientras el localizador siga vinculado. Desde 0.63.0, en cambio, un camino puede tener varias filas, una por repositorio, y `settle` elige entre ellas por el remoto. Un `path` único por identidad no puede representar eso. Hay tres opciones, y el diseño elige:
  - que el camino sólo sea único entre los localizadores sin remoto;
  - que el localizador sea el par (camino, huella);
  - que un camino compartido no sea un localizador por sí solo.
- **Cómo leer el primer commit.** Correr `git` o leer los objetos. Qué hacer con varios commits raíz y con un clone superficial, y si se guarda en caché por localizador para no pasarse de los 3 s de los hooks.
- **El archivo marcador.** No como fuente de verdad. Queda por decidir si algún día se ofrece como localizador opcional.
- **La integridad referencial.** Una regla de `doctor` más el alta al vuelo, en lugar de reconstruir `memory`. Falta definir qué hallazgos y en qué entradas.
- **Volver a importar un export de clawg después de un movimiento simple.** Se acepta que inserte de nuevo bajo su proyecto de origen, o se busca presencia en cualquier proyecto por `(agente, hash, created_at)` (`crates/clawg-core/src/memory/portability/import.rs:1130-1133`). Lo segundo tiene el riesgo de juntar un texto legítimo en dos proyectos.
- **`project_remote`.** Se mantiene como tabla o pasa a ser una vista sobre los localizadores `remote`. Hoy la leen `settle` (los remotos atados a cada fila, `path_rows_in`, `project.rs:141-151`) y la regla de `doctor` `project_remotes`.
- **Una búsqueda cuando el cwd no resuelve a ningún proyecto.** Sin alcance, como hoy, o un error.
- **Cómo responde el agente una sugerencia.** Con una herramienta nueva o con un argumento en una existente. Las dos opciones cambian `agent_contract.json`, `agent_contract.es.json`, `protocol.md`, `protocol.es.md` y `tools_es.rs`. También falta decidir si unir exige `user_answered` con la respuesta del usuario.
- **El nombre.** Si es único entre los proyectos vivos (y cómo desambigua el relleno dos carpetas `api`), o si lo desambigua el id.
- **Un save bajo el id de un proyecto borrado** (de un puente abierto o de un export). Se acepta y el proyecto vuelve marcado sin revisar, o se rechaza antes del outbox.
- **Los tipos nuevos de `memory_import_extra`**: reconstruir la tabla o agregar una tabla de diario nueva.

## Cómo se mide

Por los tests RED de cada entrega, que fallan hoy, y por el test sembrado en V26. **Aceptación:**
- la migración no cambia el `project_id` de ninguna fila de `memory`;
- todo lo guardado sigue alcanzable;
- cada unión y cada borrado de etapa 2 se deshacen con su `--undo N`, sin perder lo guardado después;
- un repo distinto en el camino de otro nunca recibe su proyecto, tampoco cuando a uno de los dos le falta `origin`, siempre que su primer commit se pueda leer;
- ningún texto, camino ni nombre de una persona o de un proyecto real entra en tests ni en documentos versionados.

Antes de la línea base de 86, se cuentan las filas sin proyecto que quedan después de colocar las del import. `rag-eval` no aplica: el cambio no toca el ranking.
