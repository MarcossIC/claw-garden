# project-registry — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 | M–L, en cinco entregas (un parche S y cuatro M) | project-reassign (77) y project-clone-identity (30), archivados: se construye sobre R1–R19 de [`specs/project`](../../../specs/project/spec.md) | archivado el 2026-10-25 con la entrega 0; las entregas 1 a 4 pasaron a [project-locators](../2026-10-27-project-locators/proposal.md) (93) ([`state.yaml`](state.yaml)) |

## Problema

El detalle, con cada lugar citado, está en la [exploración](exploration.md#problema-verificado-contra-el-código). Hoy un proyecto *es* una carpeta: su id sale del camino (`derive_project_id`, `crates/clawg-core/src/memory/identity.rs:11-15`) y `project.root_path` es `NOT NULL` (`crates/clawg-core/migrations/V9__project.sql:6-10`). De ahí salen cuatro fallas, de la más grave a la menos grave:

1. **Un repo nuevo en el camino de otro se funde con él, en silencio.** Con `assigned`, `root_id_in` devuelve el id grabado para el camino sin mirar qué hay hoy en la carpeta (`crates/clawg-core/src/memory/project.rs:119-123`, `recorded_id_in` `:103-106`). `plan_in` usa el proyecto del remoto sólo si `project_remote` ya lo conoce y, si no, el id de raíz, que es el del repo anterior (`:191-195`). `write_in` ata después el remoto del recién llegado a ese proyecto (`:253-257`), así que **todo clone posterior de ese remoto también se une al proyecto ajeno**. Sin `assigned`, `derive_project_id` da el mismo id, y el 409 sólo salta cuando el id es de *otra* raíz (`:196-206`). La lectura sin escritura (`project_id_in`, `:136-145`) y el plan del import (`plan_ids_in`, `:151-185`) siguen la misma regla. Ningún test cubre «mismo camino, otro repo». Así se pierde el aislamiento: las memorias de un repo se leen y se escriben como si fueran del otro.
2. **Una carpeta sin `origin` que se mueve o se renombra pierde su proyecto.** El camino nuevo no está grabado y recibe `candidate_id(raíz, 0)` (`project.rs:124-128`). `legacy_ids` sólo cubre el camino por el que se llegó a la misma raíz (`identity.rs:54-58`). `doctor` no lo ve, porque el id viejo sigue siendo un candidato de su raíz vieja (`crates/clawg-cli/src/doctor/checks.rs:561-585`).
3. **No existe un proyecto sin carpeta.** `--to-project` exige una fila de `project` (`check_target_in`, `crates/clawg-core/src/memory/relocate.rs:426-436`), y esa fila sólo nace de una raíz. Un `project_path` inventado se registra como una raíz normalizada a mano (`identity.rs:67-74`), cuya identidad es el hash de un camino que no existe. Por eso no se pueden colocar sin inventar carpetas las 15.709 filas que el import dejó sin proyecto en el store de producción (cifra medida allí, no releída en este repo). Mientras tanto compiten en la búsqueda de cada proyecto (`memory_project_predicate!`, `crates/clawg-core/src/memory/search.rs:107-113`).
4. **No hay dónde unir, renombrar o borrar un proyecto.** `merge_in` (`relocate.rs:884`) sólo lo dispara el registro (`project.rs:261`). Un duplicado se arregla a mano con `projects move` por filtros, que no lleva `synthesis_review`, `workspace.md`, `episodic_trajectory` ni `procedural_skill`. Además, nada garantiza que cada `memory.project_id` esté en `project`. El puente guarda los ids de cada raíz durante toda la sesión (`crates/clawg-cli/src/mcp/server.rs:874-878`) y, si el registro falla, escribe bajo el id derivado sin registrarlo (`:884-892`).

## Objetivo

Objetivo 2: separar las memorias por proyecto y también usarlas entre proyectos. Para separar hace falta una identidad que no se confunda con la carpeta en la que vive un proyecto en un momento dado. Que un proyecto quede «sin proyecto» por accidente tiene que dejar de ser la única manera de usarlo en todos lados. Lo global no cambia: una fila sin proyecto y una `user_preference` se siguen leyendo desde cualquier proyecto (`search.rs:107-113`), y `all_projects` sigue buscando en todos.

## Decisiones

Tomadas el 2026-10-24 sobre las [decisiones abiertas](exploration.md#decisiones-abiertas) de la exploración:

La decisión abierta de la entrega 0 (si el parche lleva una migración mínima) se tomó en el [diseño](design.md#d5-el-parche-no-lleva-migración): no la lleva.

1. **La identidad se separa de la ubicación (enfoque A), con el arreglo del fundido antes.** Un registro de proyectos (id estable, nombre, lápida de unión) y localizadores (camino, remoto, primer commit) que apuntan a él. Un localizador sólo vincula si lo valida una huella que no depende del camino. Los ids que ya existen (`nombre-1a2b`) se conservan y **ninguna fila de `memory` cambia de `project_id`**. V27 es aditiva y se rellena desde cada tabla que guarda un id de proyecto.
2. **El arreglo del fundido va primero y solo**: entrega 0, publicada sola como versión patch, con su test RED y una regla de `doctor` de solo lectura para los stores que ya se fundieron. Su límite conocido (el repo anterior no tenía remoto) lo cierra la huella del primer commit en la entrega 1.
3. **Alta automática.** Nada puede depender de que el usuario se acuerde de correr un comando. Las carpetas se siguen registrando solas en su primer uso (`server.rs:881`, `crates/clawg-cli/src/hook.rs:220-228`). Un proyecto sin carpeta lo crea el agente cuando guarda con un nombre de `project` que no existe, pero antes el daemon busca parecidos (nombre normalizado, localizadores). Si los hay, no crea: devuelve los candidatos para que el agente le pregunte al usuario. Ante evidencia débil (el mismo primer commit con el camino viejo todavía presente, una carpeta con el nombre de un proyecto cuyo camino ya no existe, un nombre parecido) se crea un proyecto **separado** y una sugerencia pendiente. `session-start` y `recall_context` la muestran, como hoy ACTIVE CONFLICTS, hasta que el agente la pregunta. Una respuesta «separados» se guarda y no se vuelve a preguntar. Ante la duda, separado: una unión se deshace, y separar lo fundido no se puede.
4. **El usuario revisa cuando quiere.** `clawg projects list` marca los proyectos creados solos y todavía sin revisar. `rename` cambia el nombre y nunca el id. `merge X --into Z` pasa todo lo de X a Z, y para el usuario X desaparece: no se lista, no se elige y su nombre queda libre. Por dentro queda una lápida `merged_into` que redirige, porque los puentes abiertos guardan los ids durante toda la sesión (`server.rs:874-878`) y los exports viejos llevan X. `doctor` lista los duplicados sospechados y propone el `merge` en `--dry-run` como paso siguiente.
5. **Borrar un proyecto: dos etapas.**
   - **Etapa 1.** Si el proyecto tiene memorias vivas, `delete` se rechaza y propone vaciarlo antes: `merge` hacia un candidato parecido, o `projects move`. Un proyecto vacío (sin filas no borradas en las tres tablas con proyecto) se borra.
   - **Etapa 2.** Para sacar de la memoria todo lo de un proyecto hace falta un doble chequeo, por CLI y nunca por un agente. Primero un informe: lo que se borraría por tipo, autor y estado, con las `user_preference` y `architectural_decision` aparte, y la sugerencia de vaciarlo en otro proyecto. Después, la confirmación explícita de ese mismo conjunto (ver la elección más abajo).
6. **Forma del id de un proyecto nuevo**: `slug-4hex`, compatible con `is_project_id` (`project.rs:78-87`). Se comprueba libre contra el registro, como hace `root_id_in` (`:124-128`).
7. **Vincular un camino nuevo**: es automático por remoto, como hoy. Por primer commit sólo cuando el camino viejo ya no existe (una mudanza). En cualquier otro caso queda una sugerencia.
8. **Las 15.709 filas sin proyecto esperan a este cambio**, sin carpetas provisionales. Cuando exista un proyecto sin carpeta (entrega 3), se colocan con `clawg projects move --origin-project NOMBRE --to-project PROYECTO`.

### La elección de la propuesta para la etapa 2

**Las filas quedan borradas y se pueden restaurar, como con cualquier `delete`.** La etapa 2 tiene un número de borrado y su `--undo N`, y la eliminación permanente sigue siendo un paso aparte con `clawg purge`. Hoy `purge` toma un solo id (`crates/clawg-core/src/memory/secrets.rs:181`). Gana `--project-delete N`: borra para siempre las filas que el borrado N dejó borradas, con las mismas reglas (daemon parado, sin snapshot) y los mismos avisos de snapshots y `workspace.md` que todavía guardan el texto (`secrets.rs:129-131`).

- **Cómo prueba el segundo paso que el conjunto es el mismo.** El primer paso (`clawg projects delete X --with-memories`) no escribe nada e imprime el informe y un código corto. El código es un resumen de lo que se borraría: el id del proyecto y, por cada fila de las tres tablas, su id, `status`, `deleted_at` y `update_count`, junto con sus localizadores. El segundo paso (`… --confirm CÓDIGO`) recalcula el código dentro de la transacción que escribe. Si un save llegó, una fila se editó o un localizador cambió, se rechaza sin escribir y muestra el informe nuevo. No se guarda estado: el código vence solo en cuanto el conjunto cambia.
- **Lo que tiene en cuenta:**
  - **Revivir.** Borrar la vigente de una clave revive la anterior (`plan_revival`, `crates/clawg-core/src/memory/revival.rs:42-55`; `delete_memory_core` lo corre en cada borrado, `crates/clawg-core/src/memory/admin.rs:449-453`). Las filas del proyecto se marcan juntas, y como `REPLACED_SQL` sólo mira filas no borradas (`revival.rs:73`), **no revive nada dentro del proyecto**. Una fila de afuera que un veredicto reemplazó sigue reemplazada, como con cualquier borrado.
  - **Filas de otros agentes.** Una llamada del usuario alcanza todas las filas. El informe las cuenta por autor.
  - **Saves en el outbox.** El evento lleva el proyecto (`crates/clawg-core/src/memory/events.rs:15`). Un save pendiente para el proyecto llegaría después del borrado, así que la etapa 2 se rechaza mientras haya uno, y nombra los `failed` con su `clawg outbox discard`.
  - **Pares de conflicto que cruzan a otro proyecto.** Quedan como están, contados.
  - **Conclusiones de afuera derivadas de sus filas.** Pasan a `to_review` y el informe las cuenta.
  - **Los `workspace.md` del proyecto** (`crates/clawg-core/src/memory/workspace.rs:44`). Se nombran y no se tocan.
  - **El índice de docs de sus raíces.** Es derivado del disco: se quita, se baja `index_docs` y se desvinculan los localizadores. El `--undo` los vuelve a vincular y el próximo refresco lo reconstruye.
  - **Los snapshots que guardan el texto.** Los nombra `purge`, como hoy.
  - **Un snapshot antes de escribir**, como `pre-move` y `pre-import`. El `--undo` necesita el modelo, porque un borrado quita los vectores (`admin.rs:441`) y restaurar los recalcula, como `undelete`.
- **La alternativa, una purga permanente del proyecto en un paso.** A favor: un solo comando para quien de verdad quiere que no quede nada. En contra: no se deshace salvo con un snapshot, que guarda justo el texto que se quería sacar; exige el daemon parado y reconstruir el archivo, como `purge`; y mezcla en un mismo paso el doble chequeo de la etapa 2 con lo irreversible. Separar los pasos mantiene la regla de hoy: un `delete` se deshace, y `purge` es lo único que no tiene vuelta.

## Entregas

Cada entrega se publica sola, con commits atómicos y TDD estricto. Los tests RED salen de [«Cómo se mide»](exploration.md#cómo-se-mide) de la exploración, más los que piden las decisiones de arriba.

Al archivar (2026-10-25), el cambio se partió. La entrega 0 se cerró aquí y sale en 0.63.0. Las entregas 1 a 4, con las decisiones de arriba, pasaron a [project-locators](../2026-10-27-project-locators/proposal.md) (93). Esperarlas aquí dejaba abierto un cambio cuya parte publicada ya estaba en las specs, mientras lo que queda todavía no tiene diseño.

### 0. El fundido (versión patch, sin esquema nuevo)

**Qué trae.** `root_id_in`, `plan_in`, `project_id_in` y `plan_ids_in` comparten una decisión pura: un camino grabado vale para un proyecto sólo si la huella no lo contradice. Hay contradicción cuando el `origin_key` actual de la raíz y algún remoto que `project_remote` tiene para ese proyecto son los dos conocidos y distintos. Con contradicción, la raíz recibe una fila propia en `project` y su remoto se ata a esa fila.

Cuando un camino tiene varias filas, se resuelve a la que su huella valida, y no al siguiente candidato libre. Hoy el bucle de candidatos da por tomada la segunda fila del mismo camino (`root_of_in(id).is_none()`, `project.rs:126`), así que un arreglo ingenuo abriría un proyecto nuevo en cada sesión.

Un cliente sin `assigned` recibe el 409 `project_root_collision` que ya entiende (R18), en vez del proyecto ajeno.

La regla de `doctor` lee sin escribir el `origin` actual de cada `root_path` y lo compara con los remotos grabados de su proyecto. Da un `warn` por cada proyecto con dos o más remotos grabados (lo que deja un fundido, y también un remoto renombrado), con el `origin` que hoy tiene cada una de sus raíces. La evidencia son ids y remotos. No repara nada.

**Tests RED.**
- Se registra A (`acme/a`) en `P` y después B (`acme/b`, que nunca se registró) en el mismo `P`. Hoy las dos raíces dan el mismo `project_id` y `project_remote[acme/b]` apunta a A. Con el arreglo, B tiene un proyecto propio y un tercer clone de `acme/b` en `Q` se une a B.
- `project_id_in` y `plan_ids_in` dan lo mismo sin escribir.
- Cuando A vuelve a `P`, recupera su proyecto, sin una tercera fila. B registrado dos veces conserva un solo id.
- Un cliente sin `assigned` en el mismo caso recibe el 409, sin filas movidas.
- Agregarle un primer remoto a un repo local que no tenía conserva el proyecto.
- La regla de `doctor` encuentra el caso fundido en un store sembrado y no marca un store sano.

### 1. El registro (V27)

**Qué trae.**
- **V27, aditiva**: `project_identity` (id, nombre, cómo nació, si se revisó, `merged_into`), `project_locator` (`path`, `remote`, `root_commit`; un índice único parcial sobre `(kind, value)` mientras siga vinculado) y `project_suggestion` (par, motivo, respuesta). Detalle en [Migración](#datos-guardados-migración-y-vuelta-atrás).
- **El resolvedor puro `resolve_in(ubicación, huellas, registro)`**: camino validado, después remoto, después primer commit. El primer commit vincula sólo si el camino viejo ya no existe; si no, abre un proyecto separado con una sugerencia. Si nada coincide, abre un proyecto nuevo.
- **La lectura del primer commit, fuera del camino caliente**: se lee sólo cuando el camino no está grabado o la huella lo contradice.
- **`clawg projects list`**, que marca lo creado solo sin revisar y las sugerencias pendientes, y **`clawg projects rename`**.
- **Los clientes no notan nada**: los ids que reciben son los de siempre.

**Tests RED.**
- Un repo local sin `origin`, con el mismo primer commit y el camino viejo ausente, se vincula a su proyecto. Con el camino viejo presente, abre un proyecto nuevo y deja una sugerencia.
- A sin remoto en `P` y después B con remoto en `P`, con otro primer commit, dan dos proyectos. Esto cierra el límite de la entrega 0.
- Un remoto renombrado en el mismo camino, con el mismo primer commit, deja una sugerencia en vez de un fundido o una separación muda.
- Un camino grabado que no se contradice no lee el primer commit.
- `rename` conserva el id y todo lo guardado sigue alcanzable.
- `projects list` marca lo creado solo y la sugerencia.
- El test de la base en V26 (ver [Migración](#datos-guardados-migración-y-vuelta-atrás)).

### 2. Unir, revisar y borrar

**Qué trae.**
- **`clawg projects merge X --into Z [--dry-run] [--undo N]`.** Las filas de `memory` pasan con `merge_in` (R17), las otras dos tablas con `project_rekey`, `synthesis_review` por el máximo de `reviewed_at` (su clave primaria `(agent_id, scope_key)` puede chocar, `V10__synthesis.sql:22-27`) y los `workspace.md` con `adopt_legacy` (`workspace.rs:57`). Los localizadores se reapuntan y X queda con la lápida. Antes se toma un snapshot `pre-rekey` y hay un diario propio para el `--undo`.
- **`canonical(id)`**, que sigue `merged_into` con un tope de saltos y detecta ciclos, en cada entrada del daemon que recibe un id de proyecto: guardar, buscar, contexto, sesiones, disputas, síntesis, `PATCH`, mover e importar.
- **Las respuestas a las sugerencias por CLI**: unir, o mantener separados.
- **Los hallazgos de duplicados en `doctor`**: el mismo nombre, el mismo primer commit o un id sin identidad.
- **`delete` en sus dos etapas**, con su `--undo N`, y **`clawg purge --project-delete N`**.
- **Una migración aditiva propia** para los diarios de unión y de borrado, con su test sembrado en V27.

**Tests RED.**
- `merge A --into B`, con filas en las tres tablas, `synthesis_review` en ambos y un `workspace.md` en ambos: todo queda bajo B, y `--undo N` deja el store como estaba (filas, estados, localizadores, `synthesis_review`, archivos).
- Después de la unión, un save con el id A cae en B. A no sale en `projects list`, no se elige por nombre y su nombre queda libre.
- Un recorrido de las rutas del router que reciben un id de proyecto falla si se agrega una ruta que no resuelve hacia adelante.
- Etapa 1: el `delete` de un proyecto con memorias vivas se rechaza, las cuenta y propone el `merge` hacia el candidato parecido. Uno vacío se borra.
- Etapa 2, primer paso: no escribe nada y da un código. Un save o una edición posterior invalidan ese código.
- Etapa 2, con el código correcto: borra todo, no revive nada dentro del proyecto y deja como estaban las filas de afuera que un veredicto reemplazó. Se rechaza mientras haya un save pendiente para el proyecto.
- Etapa 2, `--undo N`: restaura las filas (las vuelve a embeber) y sus localizadores.
- `purge --project-delete N` borra para siempre y nombra los snapshots y los `workspace.md` que todavía guardan el texto.

### 3. Proyectos sin carpeta y el agente

**Qué trae.**
- **Un argumento opcional `project`** (id o nombre) en las herramientas que hoy toman `project_path` y en `update_memory`. Un nombre que no existe y no tiene parecidos crea el proyecto antes de escribir el outbox. Uno con parecidos no crea nada y responde los candidatos, como un 400 que no escribe nada.
- **Las sugerencias pendientes en `session-start` y en `recall_context`**, y el camino para que el agente las responda.
- **Nombres donde hoy van ids**: `--to-project` y `PATCH project_id` aceptan nombres.
- **`clawg projects create|locate`**, para quien quiera crear o ubicar un proyecto a mano.
- **El contrato del modelo**: `agent_contract.json` y `.es.json` vueltos a bendecir, `protocol.md` y `.es.md`, y `tools_es.rs`.
- **Después de esta entrega, un paso operativo** que no es código: colocar las filas sin proyecto de producción.

**Tests RED.**
- Un `project` inexistente y sin parecidos crea el proyecto. Con un parecido (`Mi App` frente a `mi-app`) no crea nada y devuelve los candidatos.
- Una carpeta nueva con el nombre de un proyecto cuyo camino ya no existe abre un proyecto separado y deja una sugerencia que `session-start` y `recall_context` muestran hasta que se responde. Responder «separados» la quita para siempre.
- Los dos contratos cambian sólo por el argumento, y sin él el comportamiento es el de hoy.
- `search_project` sobre un proyecto sin carpeta responde su propio mensaje.

### 4. Exportar e importar

**Qué trae.**
- **La línea `project` del formato v1 gana campos opcionales**: `name`, localizadores `remote` y `root_commit`, y `merged_into`. Los caminos no viajan como identidad: siguen como `root_path` informativo y como `--map-root`.
- **El import resuelve por localizador antes que por camino y sigue la lápida.** `--project-map NOMBRE=project:ID|NOMBRE` acepta un destino sin carpeta.
- **Lo que registra un import entra al diario V26.** El `CHECK` de `memory_import_extra` (`V26__import_journal.sql:60`) no admite tipos nuevos sin reconstruir esa tabla chica; se resuelve en diseño.

**Tests RED.**
- Un export hecho antes de una unión se importa en el destino sin crear el proyecto absorbido.
- Un export de otra máquina con el mismo remoto cae en el proyecto local.
- Un proyecto sin carpeta va y vuelve por nombre.

## Alcance

1. Entrega 0 completa.
2. V27 y su relleno, el resolvedor con huella, la lectura del primer commit, las sugerencias, `projects list|rename`.
3. `merge` con lápida, diario y `--undo`, y la resolución hacia adelante. `delete` en dos etapas con `--undo`, `purge --project-delete`. Los hallazgos de `doctor`.
4. Los proyectos sin carpeta: alta automática con búsqueda de parecidos, el argumento `project` del contrato en las dos lenguas, `projects create|locate`, nombres en `--to-project` y `PATCH`.
5. Export con campos opcionales e import por localizador y lápida.
6. Las specs: deltas de `project` (R15, R16 y requisitos nuevos), `storage`, `doctor`, `mcp`, `hooks`, `portability`, `secrets` y `restore` (para `purge`). También `.claude/CLAUDE.md` donde cambie lo visible.

## Fuera de alcance

- Reescribir `memory.project_id`, reconstruir `memory` o agregarle una clave foránea real: tiene vec0, FTS y triggers (V8), y una FK rechazaría saves que hoy se aceptan (`server.rs:884-892`).
- Separar automáticamente lo que el bug ya fundió. Se detecta y se ofrece `projects move`, pero no se adivina fila por fila.
- Cambios de ranking, del piso de coseno o de `memory_project_predicate!`.
- Un archivo marcador en la carpeta del usuario como fuente de verdad.
- Cambiar `sharing.json`, el índice de docs (sigue por id de raíz) o el formato de `workspace.md`.
- Subir la versión del formato de export.
- Unir automáticamente por primer commit: un fork y una copia lo comparten, y R16 fija que un fork es otro proyecto.

## Datos guardados: migración y vuelta atrás

- **Entrega 0: sin esquema.** Lo único nuevo es una segunda fila de `project` con el mismo `root_path`, algo que `idx_project_root` no impide porque no es único (`V24__project_remote.sql:35`). **Volver a una versión anterior tiene un costo que hay que decir.** El binario anterior resuelve el camino a la fila más vieja (`project.rs:103-106`). Si el remoto del repo nuevo ya está en `project_remote`, toma ese proyecto (`:193-194`) y une a él las filas del id de raíz viejo, porque ese id es de la misma raíz (`:207-219`). Lo hace con un snapshot `pre-rekey` y con `memory_move`, así que `clawg projects move --undo N` lo devuelve. El diseño decide si alcanza con documentarlo o si el parche lleva una migración mínima para que un binario anterior rechace el store (y entonces la identidad toma el número siguiente).
- **Entrega 1: V27, aditiva.**
  - **Antes de migrar**, la apertura escribe el snapshot `pre-migration` (`crates/clawg-core/src/memory/db.rs:237`).
  - **El relleno** crea una identidad por cada id distinto de `project.id`, `project_remote.project_id`, `memory.project_id`, `episodic_trajectory.project_id`, `procedural_skill.project_id`, `memory_move.from_project` y `to_project`, `project_rekey.legacy_id` y `project_id`, `synthesis_review.scope_key` no vacío y `memory_import_extra` de tipo `project`. Así ningún id que hoy aparece en algún lado queda sin identidad, ni siquiera los que no están en `project`.
  - **El nombre** es la parte anterior al último `-`.
  - **Los localizadores**: `path` desde `project`, resuelto al proyecto que ese camino lee hoy, y `remote` desde `project_remote`.
  - **Lo creado por el relleno no se marca «sin revisar».**
  - **Ninguna fila de `memory` cambia.**
- **El test sembrado en V26.** El store tiene dos raíces que comparten remoto, una colisión de hash, filas en las tres tablas, `memory_move`, `project_rekey`, `synthesis_review`, un `project_id` sin fila en `project` y un caso fundido. Se abre con V27 y se comprueba que:
  - ninguna fila cambió;
  - cada id tiene identidad;
  - `search`, `list` y `context` devuelven lo mismo que antes;
  - `doctor` no marca nada nuevo fuera del caso fundido.
- **Las entregas 2 a 4** traen cada una su migración aditiva, con su test sembrado en la versión anterior.
- **Volver atrás del esquema.** Un binario anterior rechaza un store más nuevo sin tocarlo (`SchemaNewer`, `db.rs:443`). `clawg update` restaura en el proceso el snapshot `pre-migration` cuando el binario viejo rechaza el store (`.claude/CLAUDE.md`, `clawg update`). A mano, con `clawg restore` de ese snapshot. Como las tablas nuevas sólo se agregan, el diseño deja también el SQL que las quita, como hizo [project-reassign](../2026-10-22-project-reassign/proposal.md#datos-guardados).
- **Volver atrás de una operación.** Una unión con `projects merge --undo N` o su `pre-rekey`. Un borrado de etapa 2 con `projects delete --undo N` o su snapshot. Lo que se purgó, sólo con un snapshot.

## Compatibilidad

- **Clientes sin `assigned`.** Conservan R18. La única novedad es el 409 cuando la huella contradice el camino (entrega 0). Un id derivado que nadie conoce se registra al vuelo.
- **Puentes abiertos.** Guardan los ids durante toda la sesión (`server.rs:874-878`), y `canonical(id)` los lleva al destino de una unión.
- **Hooks.** Tienen 3 s por evento: el primer commit nunca se lee para un camino grabado y sin contradicción.
- **Export v1.** Los campos nuevos son opcionales y un lector ignora los que no conoce, así que la versión del formato no cambia.
- **Contrato MCP.** El argumento es opcional y su ausencia reproduce lo de hoy. Se vuelven a bendecir los dos contratos y la apertura de cada variante de `protocol` sigue en 2048 caracteres o menos.
- **`doctor`, chequeo `projects`.** Compara cada fila de `project` con los candidatos de su raíz (`checks.rs:561-585`). Un proyecto sin carpeta no tiene fila en `project`, y la segunda fila de un camino es un candidato de su raíz, así que ninguno de los dos se marca.

## Interacciones

- **ranking-at-scale (86).** Las filas sin proyecto forman parte del fondo que mide, y colocarlas lo cambia. Este cambio va antes ([README](../../../README.md)): se colocan antes de su línea base, y su informe dice si el fondo tenía filas sin proyecto.
- **import-job (87).** Un trabajo de importación que registre proyectos pasa por el mismo resolvedor y por el diario V26. `--project-map` gana el destino `project:ID|NOMBRE`. La forma del informe se coordina.
- **project-reassign (77).** Mover sigue siendo «cambia el proyecto de una unidad entera y nada más» (R8). `check_target_in` consulta el registro y sigue la lápida.
- **project-clone-identity (30).**
  - R14 no cambia, y R17 se reutiliza para `merge`.
  - R15 y R16 ganan la validación del camino por la huella. **R16 pierde una garantía entre la entrega 0 y la 1**: hoy un remoto nuevo de una raíz registrada conserva el proyecto (`project.rs:253-257` lo graba en el mismo). Con el parche, un `origin` *distinto* de uno ya grabado abre otro proyecto, porque sin el primer commit no se distingue un remoto renombrado de otro repo en el mismo camino. En la entrega 1 ese caso pasa a ser una sugerencia.
  - R18 se conserva. R19 pide su equivalente de V26 a V27.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| Entre las entregas 0 y 1, un repo cuyo `origin` cambia (se renombró o cambió de organización) abre un proyecto nuevo y sus memorias quedan bajo el id viejo | Nada se borra: siguen alcanzables con `all_projects`, y `projects move --from-project VIEJO --to-project NUEVO` las une (se deshace). Un warn en el log y la regla de `doctor` nombran los dos ids. La entrega 1 lo convierte en sugerencia |
| Volver a una versión anterior al parche une las filas del proyecto viejo al nuevo | Snapshot `pre-rekey`, `memory_move` y `--undo N`. El diseño decide si el parche se protege con una migración mínima |
| Una entrada del daemon olvida resolver la lápida | Una sola `canonical(id)`, un test que recorre el router y la regla de `doctor` sobre filas bajo un id con `merged_into` |
| El relleno deja un id sin identidad | El relleno recorre todas las tablas con id, el test de migración compara conteos y el alta al vuelo cubre lo que llegue después |
| Los agentes crean duplicados con nombres parecidos | Búsqueda de parecidos antes de crear, sugerencias pendientes, índice único por remoto, `doctor` y `merge` reversible |
| Una sugerencia que vuelve en cada sesión cansa | Se pregunta hasta que se responde, y «separados» queda guardado |
| La etapa 2 borra más de lo que el usuario vio | El código resume el conjunto exacto y se recalcula en la transacción que escribe. Las filas quedan restaurables hasta `purge` |
| Un `--undo` de borrado choca con un proyecto que tomó el camino mientras tanto | El índice único parcial lo detecta: el `--undo` restaura las filas y deja el camino donde está, con una sugerencia de unión |
| Leer el primer commit cuesta (historia larga, varios commits raíz, clone superficial, sin `git`) | Sólo fuera del camino caliente, y sin huella si no se puede leer. El diseño elige cómo leerlo |
| El contrato del modelo cambia | Los dos contratos se vuelven a bendecir, se revisa el diff y el argumento es opcional |
| El tamaño (M–L) | Cinco entregas publicables por separado, cada una con sus tests y commits atómicos |

## Abierto para el diseño

- **Cómo leer el primer commit.** Correr `git` o leer los objetos. Qué hacer con varios commits raíz y con un clone superficial, y si se guarda en caché por localizador para no pasarse de los 3 s de los hooks.
- **El archivo marcador.** No como fuente de verdad. Queda por decidir si se ofrece algún día como localizador opcional.
- **La integridad referencial.** Una regla de `doctor` más el alta al vuelo, en vez de reconstruir `memory`. Hace falta definir qué hallazgos y en qué entradas.
- **Volver a importar un export de clawg después de un movimiento simple.** Se acepta que inserte de nuevo bajo su proyecto de origen, o se busca presencia en cualquier proyecto por `(agente, hash, created_at)` (`crates/clawg-core/src/memory/portability/import.rs:1130-1133`), con el riesgo de juntar un texto legítimo en dos proyectos.
- **`project_remote`.** Se mantiene como tabla o pasa a ser una vista sobre los localizadores `remote`.
- **Una búsqueda cuando el cwd no resuelve a ningún proyecto.** Sin alcance, como hoy, o un error.
- **Cómo responde el agente una sugerencia.** Con una herramienta nueva o con un argumento en una existente. Las dos opciones cambian `agent_contract.json`, `agent_contract.es.json`, `protocol.md`, `protocol.es.md` y `tools_es.rs`. También queda por decidir si unir exige `user_answered` con la respuesta del usuario.
- **El nombre.** Si es único entre los proyectos vivos (y cómo desambigua el relleno dos carpetas `api`) o si lo desambigua el id.
- **Un save bajo el id de un proyecto borrado** (de un puente abierto o de un export). Se acepta y el proyecto vuelve marcado sin revisar, o se rechaza antes del outbox.
- **El parche de la entrega 0 y la vuelta atrás.** Si lleva una migración mínima.
- **Los tipos nuevos de `memory_import_extra`**: reconstruir la tabla o agregar una tabla de diario nueva.

## Cómo se mide

Por los tests RED de cada entrega, que fallan hoy, y por el test sembrado en V26. **Aceptación:**
- la migración no cambia el `project_id` de ninguna fila de `memory`;
- todo lo guardado sigue alcanzable;
- cada unión y cada borrado de etapa 2 se deshace con su `--undo N`, sin perder lo guardado después;
- un repo distinto en el camino de otro nunca recibe su proyecto;
- ningún texto, camino ni nombre de una persona o de un proyecto real entra en tests ni en documentos versionados.

Medición manual, con un store del perfil dev: la regla de `doctor` de la entrega 0 sobre una copia con un caso fundido. Antes de la línea base de 86, contar las filas sin proyecto que quedan después de colocar las del import. `rag-eval` no aplica: el cambio no toca el ranking.
