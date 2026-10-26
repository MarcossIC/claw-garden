# project-locators — Diseño

Este diseño resuelve la **entrega 1** de la [propuesta](proposal.md#1-el-registro-v27) y las decisiones de [«Abierto para el diseño»](proposal.md#abierto-para-el-diseño) que le tocan:

- el localizador `path` frente a R20 (D1);
- `project_remote` (D2);
- cómo leer el primer commit (D3);
- la integridad referencial (D7);
- el nombre (D8).

Las demás esperan a su entrega (al final). Los requisitos están en la [spec](spec.md). La [entrega 2a](#entrega-2a-unir-proyectos) (D15 a D25), la [2b](#entrega-2b-borrar-proyectos) (D26 a D37) y la [3a](#entrega-3a-proyectos-sin-carpeta-y-el-agente) (D38 a D49) están después de las decisiones de la entrega 1.

## El modelo

Un proyecto es como una persona con su DNI: el número no cambia, aunque se mude, cambie de teléfono o no tenga domicilio. Las memorias se atan al DNI, nunca a la dirección.

```
project          la referencia: id, name, born, reviewed, created_at
                 (memory, episodic_trajectory, procedural_skill y cualquier tabla nueva guardan su id)
   ▲        ▲
   │        └── project_remote   remote → project_id                 (como hoy; sirve en cualquier máquina)
   └─────────── project_path     una carpeta de esta máquina → project_id, con sus huellas
                                 (la tabla `project` de antes, renombrada)
```

Un proyecto puede tener **cero o más** carpetas y **cero o más** remotos. Ninguna de las dos cosas es el proyecto, y ninguna es obligatoria. Así queda cada caso:

| Caso | Cómo se llega al proyecto | Entrega |
|---|---|---|
| Un repositorio git con `origin` | su carpeta, y su remoto en cualquier otra carpeta o máquina | 1 |
| Un repositorio git sin `origin` | su carpeta; si se mueve, su primer commit (PL5) | 1 |
| Un clone | su remoto (R16): mismo proyecto | hoy |
| Un fork | otro remoto, así que otro proyecto (R16); el primer commit compartido no sugiere nada, porque el remoto lo contradice (PL5) | hoy |
| Un remoto renombrado o que cambió de organización | otro proyecto y una sugerencia de unir (PL6) | 1 |
| Una carpeta movida o en otra máquina | su remoto, o su primer commit si el checkout viejo ya no está | 1 |
| Una carpeta borrada | el proyecto y sus memorias siguen; la carpeta figura como ausente | 1 |
| Una carpeta sin VCS, u otro VCS | su carpeta; si se mueve, una sugerencia por nombre o `projects locate` | 3 |
| Un subproyecto de un monorepo | una carpeta declarada dentro del repositorio | 3 |
| Una idea, sin carpeta | un proyecto sin carpetas ni remotos | 3 |

## D1. `project` es la referencia, y las carpetas son opcionales

V27 renombra la tabla `project` de hoy a `project_path` y crea `project` como la tabla de referencia. La tabla de hoy nunca guardó proyectos: guarda carpetas, una por repositorio desde R20 (`settle`, `crates/clawg-core/src/memory/project.rs:126-137`). El proyecto de un clone es el id de **otra** carpeta (`project.rs:280-281`), y el puente escribe bajo ids que no son de ninguna carpeta cuando falla su registro (`crates/clawg-cli/src/mcp/server.rs:884-892`). Con el renombre, el esquema dice lo que es.

`project_path` es la pieza que la propuesta pedía para el localizador `path`, y por eso no lleva un índice único sobre el camino: es el par (camino, huellas) de la segunda salida de «Abierto para el diseño». V27 le agrega:

- `project_id`, el proyecto al que lleva;
- `remote` y `root_commit`, sus huellas;
- `git_stamp`, la marca de D3.

Su id sigue siendo la clave del índice de docs (`project_doc*`, V12), y `index_docs` sigue en ella (V13).

| Alternativa | Por qué no |
|---|---|
| Mantener `project` para las carpetas y llamar a la referencia `project_identity` | Es menos código, pero `project` seguiría queriendo decir «carpeta» para siempre, y cualquiera que lea el esquema supondría lo contrario |
| Una tabla genérica `project_locator(kind, value, …)` con carpetas y remotos | Una carpeta necesita su id (clave de docs), `index_docs` y sus huellas, y un remoto no: la mitad de las columnas quedarían NULL según el tipo. Además, un índice único sobre `(kind, value)` no puede representar varios repositorios en un camino (R20). Dos tablas, una por tipo, lo dicen sin NULL, y `project_remote` ya existe |
| Columnas de carpeta en la referencia | Un proyecto con dos carpetas (un clone, una mudanza) no entra en una fila |

**Lo que cuesta el renombre:**

- Unos 15 lugares de producción leen o escriben la tabla de hoy pensando en carpetas: `project.rs`, `relocate.rs` (`check_target_in`), `project_docs/mod.rs:505`, `portability/export.rs`, `portability/import.rs`, `portability/undo.rs` e `inspect.rs`. Pasan a `project_path`.
- Los que abren stores de versiones anteriores sin migrarlos (`inspect` para `doctor`, `export::read`, `project_id_of`) tienen que elegir la tabla según el esquema: `project_path` desde V27, y `project` antes. En un store en V27, `project` existe pero ya no tiene `root_path`.
- Unos 35 tests insertan o cuentan carpetas en `project`, y pasan a `project_path` en el mismo commit.
- `memory_import_extra` conserva el tipo `project` (su `CHECK`, `V26__import_journal.sql:60`, no admite otro sin reconstruirla). Desde V27 ese tipo quiere decir «una carpeta que el import registró».

Un binario anterior rechaza el store igual que con cualquier migración (`SchemaNewer`, `crates/clawg-core/src/memory/db.rs:443`), así que el renombre no agrega un riesgo de compatibilidad que una migración aditiva no tenga.

**`check_target_in`** (`crates/clawg-core/src/memory/relocate.rs:426-436`) pregunta si el destino está en `project`. Desde V27 eso quiere decir «es un proyecto», tenga carpeta o no, que es lo que el modelo pide (PL8). Antes, un proyecto con filas y sin carpeta no se podía usar como destino.

El primer commit **no es un localizador**: nunca vincula solo. Un fork o una copia lo comparten, y R16 fija que un fork es otro proyecto. Es una huella de la carpeta: contradice (PL4), detecta una mudanza (PL5) y sugiere (PL6). Por eso no tiene tabla ni índice único, sólo un índice parcial para encontrar las carpetas que lo tienen.

## D2. `project_remote` sigue siendo una tabla

Es el localizador que funciona en cualquier máquina: un remoto apunta a un proyecto, con la clave primaria que ya impide dos proyectos con el mismo remoto (V24:8-12). La leen `settle_root_in`, `remote_project_in`, el import y `doctor` (`project_remotes`, `crates/clawg-cli/src/doctor/checks.rs:592-630`).

| Alternativa | Por qué no |
|---|---|
| Juntarla con las carpetas en una tabla de localizadores | Ver D1 |
| Copiar el remoto a la referencia | Un proyecto tiene varios remotos cuando lo renombraron o cuando se fundió (R7 de `doctor`). La tabla ya lo representa |

Lo que cambia es el remoto **de cada carpeta** (`project_path.remote`). Hasta R20, los remotos de una carpeta eran los que `project_remote` ataba a su id. Por eso una carpeta que entró al proyecto de otra por su remoto no tenía remotos propios y nunca se contradecía, y el [diseño de 89](../archive/2026-10-25-project-registry/design.md#d1-la-huella-valida-la-fila-en-una-función-pura) aceptó ese costo. Desde V27 cada carpeta graba el suyo. Una carpeta de antes con un único remoto atado lo recibe en el relleno. Una fundida (dos o más) queda en NULL y se sigue eligiendo por sus remotos atados, como en R20: no se adivina cuál es el suyo.

## D3. El primer commit se lee con `git`, una vez por directorio git

**Cómo.** Con `git rev-list --max-parents=0 --first-parent HEAD` en la raíz. El proceso:

- no lleva las variables `GIT_*` que cambian qué repositorio se lee (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`), y lleva `GIT_TERMINAL_PROMPT=0`, `GIT_OPTIONAL_LOCKS=0` y `-c core.fsmonitor=false`;
- tiene la entrada cerrada y la salida de error descartada;
- en Windows lleva `CREATE_NO_WINDOW`. El daemon arranca con `DETACHED_PROCESS` (`crates/clawg-cli/src/client.rs:1453-1457`), y sin esa marca cada `git` abriría una consola;
- espera con `try_wait` en un hilo bloqueante hasta `ROOT_COMMIT_WAIT` (1 s), y pasado eso se corta.

No hace falta una dependencia nueva ni el feature `process` de tokio.

**`--first-parent`.** Un subárbol o una historia ajena que entra por un merge llega por el segundo padre, así que no cambia el primer commit. Sin `--first-parent`, `rev-list` devuelve todos los commits raíz alcanzables: uno nuevo aparecería al unir otra historia, y la huella cambiaría sin que el repositorio cambie. Además, recorre sólo la cadena principal.

**Cuándo** (PL3). El primer commit no depende del camino, pero leerlo cuesta lanzar un proceso y recorrer la historia. Los hooks registran en cada evento con un plazo de 3 s (`project_of`, `crates/clawg-cli/src/hook.rs:220-228`). La marca del directorio git (fecha de creación y, en Unix, dispositivo e inodo) cuesta un `stat` y dice si el directorio git es el mismo que cuando se leyó. Por eso el primer commit se lee una vez por directorio git y se graba con su marca. Así se cumple que un camino grabado no lee el primer commit.

| Alternativa | Por qué no |
|---|---|
| Leer los objetos sin `git` | Hay que resolver `HEAD`, refs empaquetadas, packfiles con deltas y zlib, y recorrer la historia. Mucho código propio para algo que `git` hace bien |
| Todos los commits raíz, como conjunto | La huella cambia cuando se une una historia ajena. Para el uso de esta huella (contradecir, mudanza, sugerir) el conjunto no da nada que `--first-parent` no dé |
| Leerlo en cada registro | Agrega un proceso por cada prompt, sobre un plazo de 3 s |
| Leerlo sólo cuando el camino no está grabado o el remoto lo contradice (lo que decía la propuesta) | Un repositorio sin `origin` reemplazado por otro sin `origin` nunca se contradice por el remoto, y es el límite que esta entrega cierra. La marca da la misma economía sin ese hueco |
| Que lo mande el cliente | El daemon decide los ids con lo que lee él (R4, `register`, `crates/clawg-cli/src/daemon/api.rs:2014-2016`), y un cliente anterior no lo mandaría |
| Seguir la lectura en segundo plano después de responder | Una mudanza o una contradicción se deciden en el registro. Un primer commit que llega después no cambia esa decisión, y para grabarlo sólo hace falta un registro posterior |

**Los límites.**

- Un repositorio que tarda más de 1 s en recorrer su cadena principal, o uno superficial, queda sin primer commit para ese directorio git y conserva la regla de R20. Casi siempre son repositorios con remoto, y el remoto decide antes.
- En Windows la marca es sólo la fecha de creación. El *tunneling* de NTFS puede darle a un directorio recreado enseguida, con el mismo nombre y en la misma carpeta, la fecha del anterior. El directorio git está adentro de la raíz, así que hace falta recrear el `.git` solo en menos de 15 s. Si pasa, se conserva la carpeta, como en R20, y no es peor que hoy.
- Un sistema Unix sin fecha de creación reutiliza inodos. Pasa lo mismo, por la misma vía.
- Una carpeta de antes de V27 graba el primer commit de lo que encuentra la primera vez que se registra después de actualizar. Si en ese camino ya había otro repositorio sin `origin`, se funde como hoy. Desde ahí queda protegida.

## D4. El remoto decide antes que el primer commit, carpeta por carpeta

`fit(carpeta, checkout)` (PL4) mira el remoto primero, y el primer commit sólo cuando el remoto no se conoce de los dos lados.

| Alternativa | Por qué no |
|---|---|
| El primer commit primero | Una rama huérfana (`gh-pages`) o una historia reescrita con `filter-repo` cambian el primer commit del mismo repositorio: lo partiría en dos. Mientras que un mismo remoto con otra historia sigue siendo el mismo proyecto para quien lo clona |
| Cualquier huella igual alcanza | Un fork en el mismo camino comparte el primer commit con el original, y R16 dice que es otro proyecto |

Con el remoto renombrado y el mismo primer commit (PL6), el remoto separa y el primer commit sugiere unir. Es la decisión 7 de la propuesta: sólo se vincula solo lo que no tiene duda.

## D5. Una mudanza se vincula sólo si el checkout grabado ya no está

La decisión 7 dice «el camino viejo ya no existe». Esta regla la afina: el checkout grabado ya no está cuando el camino no existe **o** cuando el directorio git que hay hoy en él no es el grabado (otra marca). Así se cubre una carpeta movida que deja en su lugar otro repositorio.

| Alternativa | Por qué no |
|---|---|
| Sólo «el camino no existe» | Si alguien mueve `app` a `app2` y crea otro repositorio en `app`, no se ve la mudanza y `app2` queda separado sin necesidad |
| Vincular por la marca sola (un `mv` la conserva) | Una copia entre discos o un `mv` entre sistemas de archivos cambian la marca, y entonces el primer commit hace falta igual. Y la marca sola no dice nada de una raíz sin `.git` |
| Siempre preguntar | La decisión 7 eligió vincular la mudanza sin preguntar. Separar y sugerir queda para la copia y para cuando hay más de un proyecto posible |

Quién lo mira: el daemon, en disco, sólo para las carpetas con ese primer commit. Son pocas, y sólo en un camino nuevo con el primer commit conocido. La carpeta vieja queda donde está, con su id y su índice de docs: es una forma más de llegar al proyecto. Si el checkout vuelve a ese camino, la usa otra vez.

## D6. La referencia y su relleno

`project(id, name, born, reviewed, created_at)` es lo mínimo que esta entrega usa: el nombre para `rename` y `list`, cómo nació y la marca de revisado. `merged_into` y `merged_at` llegan con la unión, en la migración propia de la entrega 2. Ahí también entra cualquier índice parcial que la lápida necesite. Agregarlos ahora sería una columna sin uso, y la entrega 2 trae una migración igual.

`born` no lleva `CHECK`. El de `memory_import_extra` (`V26__import_journal.sql:60`) obliga a reconstruir la tabla para sumar un valor (la entrega 4 lo tiene que resolver), y aquí se sabe que vienen valores nuevos: `agent` y `user` en la entrega 3. Los de esta entrega son `migrated`, `root`, `adopted` e `import`. El valor lo valida Rust.

**Qué ids reciben un proyecto en el relleno.** Los que tienen filas en las tres tablas (también las borradas) o un remoto en `project_remote`. La propuesta pedía además los ids de las carpetas, de `memory_move`, de `project_rekey`, de `synthesis_review` y de `memory_import_extra`. Se dejan afuera por dos razones:

- Un id que sólo aparece en un registro de historia es casi siempre un id viejo que una unión vació. Su proyecto sería uno vacío más en `projects list`, y un falso duplicado para la búsqueda de parecidos de la entrega 3. Si alguna vez vuelve a tener filas (un `--undo`), lo recibe por PL2.
- El id de una carpeta puede ser el de un clone, cuyas filas viven bajo el proyecto de su remoto. Su proyecto sería uno vacío con el nombre del real. Una carpeta donde nunca se guardó nada recibe su proyecto (`root`) en su próximo registro.

**El nombre** sale del id (el `{basename}` de `derive_project_id`, `crates/clawg-core/src/memory/identity.rs:11-15`). **La fecha** es la más vieja de lo que la nombra: un proyecto tiene la edad de su primera memoria o de su primera carpeta, no la de la migración.

**A qué proyecto lleva cada carpeta** (`project_path.project_id`). El relleno pone su propio id cuando ese id es un proyecto. Deja NULL en los demás casos: la carpeta de un clone, cuyo proyecto es el de su remoto, y una carpeta donde nunca se guardó nada. Desde el store no se distinguen esos dos casos: los dos tienen un id que no es un proyecto. Para saberlo hay que leer el disco. NULL quiere decir «R16», que es lo que pasaba antes de V27, y el primer registro lo graba. `projects list` lee la clave de esas carpetas en disco, como hace `doctor` (`crates/clawg-cli/src/doctor/mod.rs:196`).

## D7. Cada id que se escribe se adopta con un disparador

La propuesta dejó abierto cómo garantizar que cada `project_id` tenga su proyecto sin una clave foránea. Hoy escriben `memory.project_id`:

- el worker de ingest;
- el import;
- `move`, `undo` y `merge_in` de `relocate.rs`;
- el `PATCH`;
- el puente, que, cuando su registro falla, guarda bajo un id que nunca registró (`ClawgMcp::ids_for`, `server.rs:884-892`).

V27 agrega a `memory` dos disparadores, `AFTER INSERT` y `AFTER UPDATE OF project_id`, que hacen `INSERT OR IGNORE` del proyecto con `born = 'adopted'`. No rechazan nada. Funcionan como los de FTS (`V8__memory_fts.sql:63-90`): mantienen una tabla derivada en la misma escritura.

| Alternativa | Por qué no |
|---|---|
| Una llamada `ensure_project_in` en cada escritor | Es el riesgo que nombra la propuesta: un escritor nuevo que se olvida. El disparador cubre a los de hoy y a los que vengan |
| Reconstruir `memory` con una clave foránea | Descartado en la propuesta: vec0, FTS y disparadores (V8), y rechazaría saves que hoy se aceptan |
| Un disparador que rechace un id desconocido | Igual que la clave foránea: dejaría `failed` en el outbox el save de un puente cuyo registro falló |
| Proyectos derivados en una vista (los guardados más los ids en uso) | Un nombre derivado al leer cambia cuando aparece otro proyecto con el mismo nombre, y un nombre que cambia solo no sirve para referirse a un proyecto (entrega 3) |

`episodic_trajectory` y `procedural_skill` no llevan disparador. En producción sólo se actualizan (`crates/clawg-core/src/memory/admin.rs:263-266`, `:372-374`) y sólo el re-key de R4 les cambia el proyecto, siempre hacia el de un registro, que ya existe. El relleno cubre las filas que ya tienen.

El nombre que calcula el disparador (`CASE WHEN id GLOB '?*-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]' …`) repite la regla de `is_project_id` (`project.rs:84-93`). Un test compara el resultado de SQL con el de `project_name` en Rust sobre los mismos ids.

**`doctor`.** Con el disparador no quedan ids sin proyecto. Pueden aparecer en un store restaurado de antes de V27, pero ese store migra al abrir y el relleno lo cubre. El hallazgo de ids sin proyecto va con los duplicados, en la entrega 2.

## D8. El nombre no es único: el id desambigua

La propuesta preguntaba si el nombre es único entre los proyectos vivos y cómo desambigua el relleno dos carpetas `api`. Se elige que **no sea único**:

- El relleno, una raíz y una adopción pueden repetir un nombre: dos carpetas `api` se llaman las dos `api`, y el id (`api-1a2b`, `api-3c4d`) las distingue.
- `rename` rechaza (409) un nombre que ya tiene otro proyecto, o que es el id de otro, sin distinguir mayúsculas. Así un nombre que eligió el usuario no agrega ambigüedad.
- La entrega 3, que acepta nombres donde hoy van ids, resuelve primero por id y después por nombre. Un nombre que tienen dos proyectos es un error que los lista, como los parecidos de la decisión 3.

| Alternativa | Por qué no |
|---|---|
| Nombre único, y el relleno usa el id cuando choca | Una raíz o una adopción también tendrían que caer al id cuando el nombre está tomado. Si el id ya es el nombre que otro eligió con `rename`, el `INSERT OR IGNORE` del disparador saltaría el proyecto en silencio: hay que impedir nombres con forma de id, y eso rechaza nombres legítimos como `release-2024` (que tiene la forma de `is_project_id`). Además, un nombre automático como `api-3c4d` es un id con otra etiqueta |
| Nombre único con un sufijo (`api-2`) | Inventa nombres que nadie eligió, y el sufijo depende del orden en que se registraron |

«Su nombre queda libre» (la decisión 4 sobre `merge`) sigue valiendo en la entrega 2: X deja de contar para el 409 de `rename`.

## D9. Decidir, leer el disco y volver a decidir

`register_project` ya decide, toma el snapshot y vuelve a decidir dentro de la transacción que escribe (`Pass::WouldMove`, `project.rs:406-433`). La lectura del disco entra al mismo patrón:

1. una pasada en la transacción lee las carpetas del camino y devuelve lo que falta;
2. el daemon lo lee en un hilo bloqueante: el primer commit si ninguna carpeta tiene la marca de hoy, y la marca y la existencia de los checkouts grabados que podrían ser una mudanza;
3. la pasada siguiente decide con eso.

Si entre una pasada y otra cambió algo y falta un dato, se decide sin él: un primer commit desconocido no contradice, y un checkout que no se miró cuenta como presente, o sea separado.

La decisión es pura: `fit`, `settle` y `moved` reciben carpetas y huellas. La I/O entra por un trait, `Ground` (`checkout(root)`, `root_commit(root)`, `recorded_checkout_present(path, stamp)`). Su versión real vive en `identity.rs`, al lado de `origin_key`. Los tests de `project.rs` usan una versión falsa que cuenta las lecturas: el repositorio sin `origin`, la mudanza y la copia se prueban sin `git`, y la lectura real se prueba aparte, con repositorios de verdad (PL3).

`Registration::remote` pasa a ser el `Checkout` (clave y marca). `project_id_in`, `plan_ids_in`, `project_id_of` (`crates/clawg-core/src/memory/portability/export.rs:81-93`) y sus llamadores (`crates/clawg-cli/src/memories.rs:435-437`, `crates/clawg-cli/src/main.rs:226-228`) reciben un `Ground` en vez de la clave ya leída. El import lee las huellas de sus raíces en un hilo bloqueante: hoy `roots_here` lee la clave en el hilo del runtime (`import.rs:230-245`), y con un `git` de hasta 1 s eso no puede seguir así.

## D10. Las sugerencias

`project_suggestion` guarda el proyecto que se creó separado (`project_id`), el otro (`other_id`), el motivo (`root_commit`, `remote_changed`; la entrega 3 suma los suyos), la fecha y la respuesta. El índice único sobre `(min(project_id, other_id), max(project_id, other_id))` hace que un par se sugiera una sola vez. Una respuesta «separados» (entrega 2) queda en la misma fila, así que el par no vuelve.

Sólo las crea un registro con `assigned`:

- **El import** no las crea: su diario (V26) no las puede anotar, porque el `CHECK` de `memory_import_extra` no tiene ese tipo, y un `--undo` las dejaría colgando. La entrega 4 lo revisa junto con los tipos nuevos de ese diario.
- **Un cliente anterior** no se vincula por mudanza (R18), así que tampoco tiene qué sugerir.

## D11. `list` y `rename` pasan por el daemon

Como `projects add` y `projects move` (`crates/clawg-cli/src/projects.rs:29-66`): el daemon es el único que escribe el store, y `list` lee la clave de las carpetas sin proyecto grabado (D6) y si cada checkout grabado sigue ahí, que es trabajo de un hilo bloqueante del daemon. Dos rutas nuevas detrás del token y del guard: `GET /v1/projects` y `PATCH /v1/projects/{id}`.

**Revisado.** Un proyecto se marca revisado cuando el usuario le pone nombre, también si le vuelve a poner el mismo. Las entregas 2 y 3 suman las respuestas a sugerencias y la unión.

| Alternativa | Por qué no |
|---|---|
| Un comando `clawg projects review ID` | Es otra superficie para lo mismo. Con el mismo nombre, `rename` ya es «está bien así» |
| Leer el store sin el daemon, como `export` | `list` lee el disco por cada carpeta, y `rename` escribe. Repartirlos entre dos caminos no tiene sentido |

## D12. El import escribe sus carpetas como lo haría un registro

`plan_ids_in` cambia con el resolvedor (PL7), y una raíz que el plan vincula por mudanza recibe un proyecto que no es el id de su carpeta. Si `record_projects` siguiera insertando sólo `(id, root_path, created_at, index_docs)` (`import.rs:1203`), esa carpeta quedaría con `project_id` NULL, y R16 la leería con su propio id: otro proyecto que el del plan. Por eso inserta en `project_path` también `project_id`, `remote`, `root_commit` y `git_stamp`, y crea en `project` el proyecto que falte, con `born = 'import'`.

Su `--undo` borra la carpeta (`crates/clawg-core/src/memory/portability/undo.rs:391-397`) y con ella las columnas nuevas. El proyecto que el import creó queda, vacío: la entrega 2 borra los proyectos vacíos y la entrega 4 lo anota en el diario.

## D13. Un proyecto que nace de una carpeta toma su id

Un proyecto nuevo de una carpeta (PL4.3) tiene el mismo id que ella, y una carpeta nueva recibe el primero de sus candidatos que no es el id de otra carpeta, como hoy (R15). Desde ahí son independientes: la carpeta se puede mover o borrar y el proyecto conserva su id. Una carpeta posterior del mismo proyecto (un clone, una mudanza) tiene su propio id.

| Alternativa | Por qué no |
|---|---|
| Un id nuevo para cada proyecto, distinto del de su carpeta | Un cliente anterior deriva el id de la carpeta y escribe bajo él (R18). Con ids distintos, ese cliente escribiría bajo un id que no es el proyecto, y R2 («el id de una raíz canónica es el de siempre») dejaría de valer |

Los ids nuevos sin carpeta (`slug-4hex`, decisión 6) son de la entrega 3.

## D14. Lo que esta entrega deja preparado para las carpetas opcionales

Nada en esta entrega supone que un proyecto tiene carpeta:

- `projects list` muestra los proyectos sin carpeta;
- un movimiento acepta un proyecto sin carpeta como destino (PL8);
- una carpeta borrada deja al proyecto y sus memorias como estaban.

Lo que la entrega 3 suma encima:

- **Un proyecto sin carpeta.** Es una fila de `project` sin filas en `project_path` ni en `project_remote`. El esquema ya lo admite.
- **Un subproyecto de un monorepo.** Hoy la raíz sube hasta el primer `.git` (R1), así que `apps/web` es parte del repositorio. Hay que declararlo, porque el registro automático no puede adivinarlo. Una carpeta de `project_path` puede ser cualquier directorio, no sólo una raíz git: la resolución elegiría la carpeta registrada más profunda que contiene al camino, y el cliente tendría que mandar el camino crudo además de la raíz. Un clone del monorepo en otra máquina necesita el remoto más la subcarpeta, y eso cambia la clave de `project_remote` en la migración de esa entrega.
- **Una carpeta sin VCS, u otro VCS.** Se reconoce sólo por su camino. Si se mueve, queda la sugerencia por nombre de la decisión 3 o `projects locate`. Una huella de otro VCS (el primer cambio de Mercurial, por ejemplo) sería una columna más de `project_path`. Como `born` y los motivos no llevan `CHECK`, nada obliga a reconstruir una tabla.

## Costos en el camino caliente

- **Un hook en una raíz grabada** suma un `stat` del directorio git y una comparación. No corre `git`.
- **Un camino nuevo o un clone nuevo** suma un `git rev-list` de como mucho 1 s, una vez.
- **Una mudanza posible** suma un `stat` por cada carpeta con el mismo primer commit.
- **El import** suma por raíz lo mismo que un registro, en un hilo bloqueante.

## Volver atrás

- **El esquema.** Un binario anterior rechaza un store en V27 sin tocarlo (`SchemaNewer`, `db.rs:443`). La vuelta atrás es el snapshot `pre-migration` que escribe la apertura (`db.rs:237`), y `clawg update` lo restaura en el proceso. Para quitar V27 a mano conservando lo guardado después, con un `sqlite3` 3.35 o más nuevo y el daemon parado:

  ```sql
  DROP TRIGGER memory_project_ai;
  DROP TRIGGER memory_project_au;
  DROP TABLE project_suggestion;
  DROP TABLE project;
  DROP INDEX idx_project_path_root_commit;
  ALTER TABLE project_path DROP COLUMN git_stamp;
  ALTER TABLE project_path DROP COLUMN root_commit;
  ALTER TABLE project_path DROP COLUMN remote;
  ALTER TABLE project_path DROP COLUMN project_id;
  ALTER TABLE project_path RENAME TO project;
  DELETE FROM refinery_schema_history WHERE version = 27;
  ```

  Las carpetas que una mudanza o una contradicción agregaron son filas como las de R20, y un binario anterior las lee. Una raíz vinculada por mudanza vuelve a leerse con su propio id: sus memorias quedan bajo el proyecto viejo, y se alcanzan con `all_projects` o con `projects move`.
- **Lo que decide el registro.** Nada se mueve por una huella, salvo las uniones de R4 y R17, que ya tienen su `--undo`. Una mudanza sólo une los ids viejos de la raíz nueva y el id de su carpeta, que no suelen tener filas, y lo hace con el snapshot `pre-rekey` y su número de movimiento, como hoy.

## Lo que espera a otra entrega

| Decisión abierta de la propuesta | Entrega |
|---|---|
| El archivo marcador | Ninguna: sigue fuera de alcance |
| Volver a importar un export de clawg después de un movimiento simple | 4 |
| Una búsqueda cuando el cwd no resuelve a ningún proyecto | 3a ([D44](#d44-el-argumento-en-el-puente-y-las-búsquedas-sin-proyecto)) |
| Cómo responde el agente una sugerencia, y si unir exige `user_answered` | 3a ([D45](#d45-el-agente-responde-una-sugerencia-con-answer_suggestion)); la respuesta por CLI es de la 2a, D22 |
| Un save bajo el id de un proyecto borrado | 2b ([D27](#d27-un-proyecto-borrado-vuelve-al-usarse-por-disparadores)) |
| Los tipos nuevos de `memory_import_extra` | 4 |
| Los subproyectos de un monorepo y las carpetas sin VCS | las carpetas sin VCS, 3a (D46, D48); los subproyectos, 3b (D14, D49) |

# Entrega 2a: unir proyectos

Esta parte resuelve la **entrega 2a** de la [propuesta](proposal.md#2-unir-revisar-y-borrar): unir con lápida y `--undo`, `canonical(id)`, las respuestas a las sugerencias por CLI y los hallazgos de `doctor`. Los requisitos están en la [spec](spec.md#entrega-2a-unir-proyectos) (PL11 a PL20).

## D15. La lápida queda en `project`, y la unión tiene su propio diario

X no se borra de `project`: queda con `merged_into`. Hay tres razones:

- los puentes abiertos guardan sus ids toda la sesión (`server.rs:860-897`);
- los exports viejos y los registros de historia (`memory_move`) nombran a X;
- sin su fila, el próximo save bajo X lo volvería a crear vacío con el disparador de PL2, como `adopted`. Sería justo el duplicado que la unión quería quitar. Con la fila, el `WHERE NOT EXISTS` del disparador no hace nada.

| Alternativa | Por qué no |
|---|---|
| Borrar X y guardar el desvío en una tabla `project_alias` | Es otra tabla que consultar para el mismo resultado, y sin la fila de X el disparador lo vuelve a adoptar |
| Reescribir cada referencia a X | Los puentes abiertos y los exports no se pueden reescribir |

**El diario.** `project_merge` guarda una fila por unión. `project_merge_change(merge_id, kind, a, b)` guarda cada cosa que la unión cambió, con lo que había antes, como `memory_import_extra` (`V26__import_journal.sql:55-60`). No lleva `CHECK` sobre `kind`, por lo que aprendió D6. Las filas de `memory` no van ahí: su diario es `memory_move` y `memory_move_supersede`, bajo el `move_id` de la unión.

| `kind` | `a` | `b` |
|---|---|---|
| `tombstoned` | X, cuando la unión lo hizo lápida (una reparación no lo anota) | — |
| `trajectory`, `skill` | el id de la fila | — |
| `review` | el agente de una fila de `synthesis_review` de X | su `reviewed_at` |
| `review_target` | el agente de la fila de Z que tocó | el `reviewed_at` que tenía, o NULL si no había |
| `folder` | el id de una carpeta que apuntaba a X | — |
| `remote` | un remoto que apuntaba a X | — |
| `tombstone` | una lápida que apuntaba a X | — |
| `suggestion` | el `rowid` de una sugerencia que la unión respondió | — |
| `reviewed` | Z | su marca de revisado de antes |
| `workspace` | el segmento del agente de una copia hecha | — |

## D16. Cada lápida apunta a un proyecto vivo

Unir B en C pasa a C las lápidas que apuntaban a B (PL13, paso 5). Así `canonical` es una sola lectura por clave primaria, y una cadena nunca tiene más de un salto. El `--undo` respeta ese orden: no se deshace A→B mientras B sea una lápida (`merged_later`), porque las filas de A ya no están en B sino en C.

`MAX_MERGE_HOPS` y la detección de ciclos son una defensa para un store editado a mano o restaurado de un estado roto. En ese caso, `canonical` devuelve el id del que partió y `doctor` lo nombra (`broken_merge`), en lugar de fallar cada pedido.

| Alternativa | Por qué no |
|---|---|
| Dejar cadenas largas y recorrerlas | Cada pedido paga el recorrido, y deshacer un eslabón del medio deja cadenas que apuntan a una lápida revivida |
| Resolver en el SQL de cada lectura (`COALESCE((SELECT merged_into …), project_id)`) | Es una subconsulta por fila en la búsqueda. Además, el camino del `workspace.md` y las sesiones se resuelven en Rust de todos modos |

## D17. `canonical` corre al entrar para leer, y en la transacción que escribe para escribir

- **Lecturas** (buscar, contexto, listar, disputas, síntesis, sesiones): al entrar. Una lectura que se cruza con una unión lee antes o después, y las dos respuestas están bien.
- **Escrituras que mueven** (`move`, `PATCH`, el `--undo` de un movimiento, el import y el registro): dentro de la transacción que escribe. El destino puede volverse una lápida entre la entrada y la escritura, y las filas quedarían bajo ella. `check_target_in` (`relocate.rs:427-438`) ya corre en esa transacción, y pasa a ser `target_in`, que devuelve el destino vivo.
- **El worker:** al empezar a procesar un evento, y la unión no corre mientras haya un save pendiente para X o para Z. El worker lee el proyecto varias veces antes de la transacción que termina el save: las filas de la clave (`worker.rs:694`), la dedup (`:598`) y el detector (`:928`), cada una en su propia llamada a la conexión. Una unión entre dos de ellas partiría la decisión. La fila del outbox existe desde que el save se acepta (`runtime.rs:250`) hasta la transacción que lo termina de guardar. Por eso, si la transacción de la unión no ve filas pendientes de X ni de Z, ningún save de esos proyectos está entre esas llamadas. Un save aceptado con X justo antes de la unión, cuya fila se escribe después, llega al worker después de la unión, y su `canonical` da Z.

`SAVES_WAIT` (5 s) evita que un save en curso haga fallar la unión: lo normal es que tarde milisegundos. Un save que espera el modelo puede tardar un minuto entre intentos (`worker.rs`), y entonces la unión responde `saves_pending` en lugar de esperar sin fin.

| Alternativa | Por qué no |
|---|---|
| Un disparador que, después del insert, pase la fila a Z | Escribe cada fila dos veces y corre los disparadores de FTS dos veces. Además, la dedup, la clave y el detector ya habrían decidido contra X |
| Pausar el worker durante la unión | El worker no tiene pausa, y una pausa mientras un save espera el modelo frenaría todos los saves |

**Las sesiones** (`crates/clawg-cli/src/daemon/sessions.rs:33-43`) se registran y se buscan por `canonical`. Una que se registró bajo X antes de la unión no se encuentra bajo Z hasta que el hook la registra otra vez, y eso pasa en cada prompt (`hook.rs:246`).

## D18. Unir pasa a Z todo lo que está bajo X

`merge_in` (R17, `relocate.rs:885-906`) deja en su lugar una unidad que tiene filas en un tercer proyecto (`decide_merge`, `:688-699`). En un registro eso está bien: el id viejo sigue siendo un proyecto vivo, y sus filas se alcanzan con `all_projects`. En una unión, X pasa a ser una lápida. Una fila que quedara bajo X no estaría en ninguna lista ni en ninguna búsqueda. Por eso la unión pasa a Z la parte de la unidad que está en X. Una unidad que ya estaba partida entre X y W queda partida entre Z y W, y no queda peor que antes.

`decide_merge` recibe la regla como argumento: lo que toca otro proyecto se deja (`Span::Keep`, el registro) o se lleva la parte de X (`Span::Carry`, la unión). `reconcile`, `supersede_in` y `apply_in` no cambian.

| Alternativa | Por qué no |
|---|---|
| Rechazar la unión mientras haya una unidad partida | Ninguna herramienta junta una unidad partida: `projects move` también la deja (`spans_projects`). La unión quedaría bloqueada para siempre |
| Llevar la unidad entera, también sus filas de W | Movería filas de un tercer proyecto que nadie nombró |

## D19. `synthesis_review` queda con la revisión más vieja

La propuesta pedía el máximo de `reviewed_at`. Esa fecha es el límite de lo que `context` cuenta como pendiente de `synthesize` (`unreviewed_count`, `synthesis.rs:282`; `conclusions_to_review_count`, `:307`).

- Con el máximo, lo de X entre las dos fechas no se contaría nunca, y el aviso se perdería sin que nadie lo note.
- Con el mínimo, lo de Z entre las dos fechas se cuenta otra vez. Eso da un aviso de más, que se responde con un `synthesize`.

Perder un aviso es perder información, y un aviso de más cuesta una llamada. Por eso queda el mínimo.

## D20. Los `workspace.md` se copian después de la transacción, como hace `adopt_legacy`

La unión copia como `adopt_legacy` (`workspace.rs:57-98`): cuando Z no tiene archivo, copia el de X, y cuando tiene uno, deja los dos y los nombra. El de X nunca cambia. Un archivo no entra en una transacción de SQLite, así que se copia después de confirmarla y las copias se anotan en una segunda transacción corta. Si el proceso muere entre las dos, queda una copia sin anotar. El `--undo` no la borra, y ese es el lado seguro.

`adopt_legacy` pasa a devolver lo que hizo (`Adopted { copied, kept_beside }`). El registro lo ignora, como ahora.

El `--undo` borra una copia sólo si su parte del usuario sigue siendo la del archivo de X. La sección AUTO se regenera en cada `context` (`regenerate_auto_section`, `workspace.rs:108`), así que comparar el archivo entero conservaría casi todas las copias. La parte del usuario es lo único que escribe la persona.

| Alternativa | Por qué no |
|---|---|
| Copiar antes de confirmar la transacción | Si la transacción falla, queda una copia sin unión |
| Mover el archivo de X | Si Z ya tenía uno, se pierde uno de los dos. Y el `--undo` tendría que devolverlo |
| Juntar las dos partes del usuario | La persona puede querer una sola. Por eso `adopt_legacy` no las junta |

## D21. Un snapshot `pre-merge`, y `pre-undo` para deshacer

La propuesta nombraba `pre-rekey`, que es el que escribe un registro (`project.rs:899`). Con un motivo propio, `Reason::PreMerge`, la persona encuentra el snapshot de la unión entre los backups. Además, como los automáticos se rotan por motivo (`AUTO_KEEP`, `backup.rs:289`), las uniones no desplazan los snapshots de los registros. El `--undo` usa `pre-undo`, como el de un import (`portability/undo.rs:829`).

## D22. Las respuestas a las sugerencias

- **Una unión** responde `merged` las sugerencias pendientes cuyos dos lados pasan a ser un solo proyecto.
- **`keep-apart`** responde `apart`. Si no había ninguna sugerencia, graba un par con motivo `user`, y el índice único del par (V27) hace que el `INSERT OR IGNORE` del registro (`project.rs:770-772`) no lo vuelva a sugerir.
- **Los lados se leen por `canonical`.** Después de unir X en Z, una sugerencia (X, W) es (Z, W). No se reescribe en el store por dos razones: el índice único podría chocar con un (Z, W) que ya existe, y el `--undo` tendría que volver a reescribirla. La lista junta las dos.
- **Revisado.** `keep-apart` marca los dos proyectos, y la unión marca Z, como D11 dejó dicho.

| Alternativa | Por qué no |
|---|---|
| Responder por el id de cada sugerencia | Las sugerencias no muestran un id: lo que la persona ve en `projects list` es el par |

## D23. Lo que administra proyectos no sigue la lápida

- **`rename`** de X después de la unión renombraría Z por su id viejo, y eso sorprende. El 409 nombra Z y deja que la persona decida.
- **`keep-apart`** y **el destino de una unión**: la persona nombró un proyecto que ya no existe.
- **`from_project`**: X ya no tiene filas, y seguirlo hasta Z movería todo Z.
- **`projects move --undo`** del movimiento de una unión devolvería las filas a una lápida (`part_of_merge`).

## D24. Un recorrido de texto del router

El router es una sola función, con un `.route(…)` por camino (`crates/clawg-cli/src/daemon/api.rs:1029-1069`), y axum no lista sus rutas en tiempo de ejecución. Un test lee la fuente, como `tests/conventions.rs` lee los workflows, saca cada camino con sus métodos y lo compara con una tabla del test. La tabla dice el campo con el proyecto de cada ruta (`body.project_id`, `query.project`, …) o que no tiene ninguno. Una ruta que no está en la tabla hace fallar el test. El segundo test usa la misma tabla para mandar el id de una lápida a cada ruta que recibe uno.

| Alternativa | Por qué no |
|---|---|
| Un extractor tipado `ProjectId` que resuelva la lápida | Los ids vienen dentro de cuerpos JSON y consultas de formas distintas, y serde no llega al store |
| Un middleware que reescriba los cuerpos | Parsea cada cuerpo dos veces, y aun así tiene que conocer cada campo |

## D25. Los hallazgos de `doctor`

`inspect` suma hechos, cada uno leído sólo donde el esquema lo tiene (`Has`, `crates/clawg-core/src/memory/inspect.rs:190`):

- los proyectos con su nombre, `created_at` y `merged_into`;
- el primer commit, el remoto y el proyecto de cada carpeta;
- los pares de sugerencias respondidas;
- los ids con filas en cada una de las tres tablas.

`same_first_commit` usa la regla de `fit` sobre el remoto: un fork (los dos con remoto, distintos) no es un duplicado (R16). El paso propone unir el más nuevo en el más viejo, que suele ser el que tiene la historia, y el mensaje dice que la otra dirección también sirve. Ningún paso pide confirmación, porque una unión en seco no escribe nada.

## Costos en el camino caliente

- Cada pedido que recibe un proyecto suma una lectura por clave primaria de `project`. El puente hace un pedido así en cada herramienta.
- Cada save suma esa misma lectura en el worker.
- Una unión lee y escribe en una transacción las filas de X y su diario. No necesita el modelo, así que no embebe nada.

## Volver atrás de la entrega 2a

- **El esquema.** Un binario anterior rechaza un store en V28 sin tocarlo (`SchemaNewer`), y la vuelta atrás es su snapshot `pre-migration`. Para quitar V28 a mano, primero hay que deshacer cada unión (`clawg projects merge --undo N`). Si no, cada lápida pasa a ser un proyecto vivo y vacío, sus filas siguen en Z y un puente con su id escribe bajo ella. Después, con un `sqlite3` 3.35 o más nuevo y el daemon parado:

  ```sql
  DROP TABLE project_merge_change;
  DROP TABLE project_merge;
  DROP INDEX idx_project_merged_into;
  ALTER TABLE project DROP COLUMN merged_at;
  ALTER TABLE project DROP COLUMN merged_into;
  DELETE FROM refinery_schema_history WHERE version = 28;
  ```
- **Una unión.** Con `clawg projects merge --undo N`, o con su snapshot `pre-merge`.

# Entrega 2b: borrar proyectos

Esta parte resuelve la **entrega 2b** de la [propuesta](proposal.md#2-unir-revisar-y-borrar): `delete` en dos etapas con su `--undo`, y `purge --project-delete`. Los requisitos están en la [spec](spec.md#entrega-2b-borrar-proyectos) (PL21 a PL31). La persona eligió tres cosas el 2026-10-25: que un proyecto borrado vuelva al usarse (D27), que la purga saque todo el conjunto (D34) y que el `--undo` deje borrado lo que `undelete` rechazaría (D33). Lo demás se le presentó y no lo objetó.

## D26. Un proyecto borrado queda en `project`, con `deleted_at`

La fila de X no se borra, por tres razones:

- **Sus filas borradas siguen bajo su id.** Sin la fila de X, `doctor` las nombraría huérfanas, porque `project_orphans` cuenta también las borradas (`IdRows`, `crates/clawg-core/src/memory/inspect.rs:253`).
- **`clawg undelete` dejaría una fila viva bajo un id sin proyecto.** Restaurar no dispara la adopción: `memory_project_au` sólo corre con `UPDATE OF project_id` (`V27__project_registry.sql:96`).
- **El `--undo` tendría que volver a crear la fila**, con su nombre, su `born` y su `created_at`, que ya no estarían en ningún lado.

La lápida de D15 ya mostró este camino: la fila queda, y lo que la nombra decide qué hacer con ella.

| Alternativa | Por qué no |
|---|---|
| Borrar la fila de `project` | Las tres razones de arriba. Además, el próximo save desde un puente abierto la volvería a crear como `adopted`, sin nombre ni historia |
| Borrarla después de la purga | Un borrado purgado todavía se nombra en el diario (`project_delete.project_id`). Y un save posterior la adoptaría como si nunca hubiera existido |

## D27. Un proyecto borrado vuelve al usarse, por disparadores

Lo eligió la persona. Un proyecto borrado no espera un comando para volver: vuelve, sin revisar, en cuanto recibe una fila viva. Es la misma regla de alta automática de la decisión 3 de la propuesta, y se cumple igual que la adopción de D7: con disparadores en `memory`, así vale para cualquier escritor.

```sql
-- memory_project_ai y memory_project_au conservan su INSERT … WHERE NOT EXISTS y suman:
UPDATE project SET deleted_at = NULL, reviewed = 0
 WHERE id = new.project_id AND deleted_at IS NOT NULL AND new.deleted_at IS NULL;

CREATE TRIGGER memory_project_restored AFTER UPDATE OF deleted_at ON memory
WHEN new.project_id IS NOT NULL AND old.deleted_at IS NOT NULL AND new.deleted_at IS NULL BEGIN
    UPDATE project SET deleted_at = NULL, reviewed = 0
     WHERE id = new.project_id AND deleted_at IS NOT NULL;
END;
```

**Por qué las carpetas y los remotos siguen vinculados.** El mismo checkout tiene que llevar al mismo proyecto: eso es lo que dicen sus huellas (D4). Si el borrado los desvinculara, la carpeta caería en su propio id de raíz (`project_of`, `crates/clawg-core/src/memory/project.rs:408`). En el caso común, ese id es el de X (D13), y X está borrado. Habría que inventar una regla para darle otro.

**Por qué registrar no lo revive.** Los hooks registran la raíz en cada prompt (`project_of`, `crates/clawg-cli/src/hook.rs:220`). Si registrar reviviera, abrir una sesión en la carpeta deshacería el borrado antes de que nadie guarde nada. En cambio, `write_in` sigue haciendo `INSERT OR IGNORE INTO project` (`project.rs:765`): la fila existe, así que no hace nada, y la respuesta nombra X. Las lecturas bajo X no encuentran filas vivas, y eso está bien.

**Por qué el `--undo` pone `deleted_at` en NULL antes de restaurar filas.** Así el disparador encuentra a X vivo y no le toca `reviewed`: deshacer deja a X como estaba, no «sin revisar».

| Alternativa | Por qué no |
|---|---|
| La carpeta empieza otro proyecto, que era lo que decía la propuesta | Hace falta una regla nueva para elegir un id que no sea el de la raíz. Un save con el id viejo recibiría 409 `project_deleted`, y el puente tendría que volver a registrar la raíz. Hasta entonces, en una misma sesión, el hook (que registra en cada prompt) y el puente apuntarían a proyectos distintos. Y el `--undo` dejaría dos proyectos para la misma carpeta, con una sugerencia de unión |
| Rechazar los saves bajo un proyecto borrado | Un puente abierto fallaría en cada save hasta reiniciarse, y al reiniciar el registro tendría que dar algún proyecto: se vuelve a la alternativa de arriba |
| Revivir en el código de cada escritor | Son el worker, el import, el `--undo` de un movimiento, el restore y el registro. Olvidar uno deja una fila viva bajo un proyecto que no se lista. Es justo lo que D7 evitó con un disparador |

## D28. El diario del borrado, en tablas con tipos

- `project_delete`: una fila por borrado.
- `project_delete_row(delete_id, tbl, row_id, deleted_at)`: cada fila del conjunto, con el `deleted_at` que tenía antes. NULL quiere decir que la borró este borrado.
- `project_delete_folder(delete_id, root_id)`: las carpetas a las que les bajó `index_docs`. Sólo se anotan las que lo tenían en 1, así que el `--undo` sabe a cuáles subírselo.

La unión usa una tabla genérica (`project_merge_change`, D15), porque cambia una decena de cosas distintas. El borrado, en cambio, cambia filas, y el `--undo` y la purga las buscan por id. Con `a TEXT` cada búsqueda necesitaría un `CAST`, que no usa índices. El diario del import hizo lo mismo con `memory_import_row` (`V26__import_journal.sql`).

`project` no lleva `delete_id`: el borrado abierto de X se encuentra por el índice de `project_delete.project_id`. Un proyecto puede borrarse, volver al usarse y borrarse otra vez, y entonces tiene varios borrados abiertos (D33).

## D29. El código resume el conjunto y se recalcula en la transacción que escribe

**La forma canónica.** El texto que se pasa por SHA-256 (`sha2`, que ya es dependencia de `clawg-core`) es:

- el id de X;
- los ids de raíz de sus carpetas, ordenados;
- sus remotos, ordenados;
- por cada tabla, en este orden (`memory`, `episodic_trajectory`, `procedural_skill`), y por cada fila ordenada por id: el id, el `status`, el `deleted_at` y el `update_count`.

Todo va separado por saltos de línea. El código son los primeros 8 hex. No es un secreto: es una prueba de que la persona vio este conjunto. Dos conjuntos distintos con el mismo código tienen una probabilidad de 1 en 4.300 millones.

**Qué lo cambia:**

| Cambio | Qué campo lo hace cambiar |
|---|---|
| un save, un import o un movimiento que trae una fila | una fila nueva en el conjunto |
| un movimiento que se lleva una fila | una fila menos |
| un `PATCH` de importancia o tipo | `update_count` (`admin.rs:261-267`) |
| un veredicto o un save bajo una clave | `status` |
| borrar una fila suelta o restaurarla | `deleted_at` (restaurar también sube `update_count`, `admin.rs:566`) |
| un registro que vincula una carpeta o un remoto | los localizadores |

`last_modified_at` no entra: todo lo que lo cambia también cambia un campo de la lista. El texto tampoco entra: nada lo edita después de guardado, y hashear miles de textos cuesta.

**Sin estado.** Las pasadas son las de la unión (`retrying`, `crates/clawg-core/src/memory/merge.rs:351`): una pasada en una transacción que no se confirma decide, después se escribe el snapshot, y una segunda pasada vuelve a decidir y escribe. El código se calcula en las dos. Si no coincide en la primera, responde `code_changed` sin snapshot. Si deja de coincidir entre el snapshot y la escritura, la transacción se deshace, y el snapshot queda para que lo rote la poda.

| Alternativa | Por qué no |
|---|---|
| Un token guardado con vencimiento | Es estado que hay que limpiar, y aun así habría que recalcular el conjunto para saber si cambió |
| Un código más largo o más corto | Más largo no se puede copiar a mano. Con 4 hex (1 en 65.536), dos conjuntos distintos se confundirían con facilidad |

## D30. El conjunto se marca junto, y nada revive

La transacción hace un `UPDATE … SET deleted_at = ?now WHERE project_id = X AND deleted_at IS NULL` por tabla, saca los vectores de esas filas y después corre `release_challengers` (`dispute.rs:197`) por cada fila de `memory` que borró.

No llama a `delete_memory_core` (`admin.rs:413`) por fila. Esa función revive en cada borrado (`revive_after_delete_of_piece`), y `REPLACED_SQL` busca las filas vivas que la fila borrada reemplazó (`revival.rs:67-74`). Si se borra fila por fila, una versión vieja todavía sin marcar revive cuando se borra la nueva. En cambio, con todas marcadas antes, `REPLACED_SQL` no encuentra ninguna del proyecto. Lo que se saltea no cambia el resultado:

- una fila de afuera que una fila del proyecto reemplazó por veredicto queda reemplazada, como en cualquier borrado (`hidden`);
- una de afuera reemplazada con causa `revision` desde otro proyecto tampoco revive, porque `REPLACED_SQL` sólo la cuenta como revisión si está en el mismo proyecto.

Saltear `revive_after_delete` da por eso el mismo resultado.

| Alternativa | Por qué no |
|---|---|
| `delete_memory_core` fila por fila, de la más nueva a la más vieja | Depende del orden de las cadenas de versiones, que pueden cruzar saves. Es una transacción por fila, y revisa el alcance de un lector que acá no hay |

## D31. Una respuesta que leyó el conjunto es un informe

Las respuestas `not_empty` y `code_changed` tienen que traer los conteos, los candidatos y el código nuevo, en el idioma de la persona. En cambio, las frases de error del daemon están en inglés y la CLI las enmarca (`.claude/CLAUDE.md`, «the sentences the daemon returns in an error»), y `ErrorBody` (`crates/clawg-cli/src/daemon/api.rs:118`) no tiene lugar para un informe. Por eso las dos responden 200 con el informe y `refused`, y la CLI sale con 1. Los 409 quedan para lo que no lee el conjunto: una lápida, un proyecto ya borrado, saves pendientes y los estados del `--undo`.

| Alternativa | Por qué no |
|---|---|
| 409 con el informe en un campo nuevo de `ErrorBody` | Es un campo que sólo usa una ruta, en el sobre de todas |
| 409 con todo en `detail` | La CLI no podría armar los comandos con los ids de los candidatos sin parsear una frase en inglés |

## D32. El índice de docs se quita, y la purga lo vuelve a quitar

El índice de docs se deriva del disco, pero guarda pasajes de los archivos del proyecto. Un proyecto borrado no los necesita, y la purga promete que no queda nada de él.

**Por qué bajar `index_docs`.** Si sólo se borraran las filas del índice, la próxima `search_project` de un puente abierto en esa carpeta lo reconstruiría: un índice de más de un minuto lanza un refresco (`POST /v1/projects/docs/search`). Con `index_docs` en 0, esa búsqueda responde 403 hasta que algo vuelva a pedirlo. Lo vuelven a pedir el `--undo` o un registro con `index_docs`: un puente que arranca en esa carpeta, o `projects add`.

**La carrera.** Un refresco en curso escribe cada doc en su propia transacción (`replace_doc`, `crates/clawg-core/src/memory/project_docs/mod.rs`), y bajar `index_docs` no lo frena. Por eso la purga vuelve a sacar lo que haya bajo las raíces del borrado que siguen en 0. Hasta la purga, lo que un refresco haya escrito no se alcanza, porque una búsqueda con `index_docs` en 0 responde 403.

Sacar el índice de una raíz es un `delete_doc` (`mod.rs:597`) por cada archivo de `project_doc_file` bajo ese id, en la transacción del borrado.

| Alternativa | Por qué no |
|---|---|
| Esperar a que termine el refresco (`DocIndex::wait_idle`) | El core no tiene el `DocIndex`, y esperar no frena un refresco que empiece después |
| Dejar el índice | Contradice la purga, y un proyecto borrado no lo usa |

## D33. Deshacer recalcula los vectores antes de la transacción

Restaurar quita el vector borrado y pide uno nuevo, igual que `undelete`, así que un `--undo` con filas necesita el modelo.

- **Los vectores se calculan fuera de la transacción**, en tandas de 32 con `embed_batch`, como el import (`EMBED_BATCH`, `crates/clawg-core/src/memory/portability/import.rs:68`). El texto es el de cada tabla: `embedding_text(context, content)` para `memory`, `task` para una trayectoria y `skill_embedding_text` para una skill (`maintenance.rs:463`).
- **Las filas se escriben en una transacción**, como la unión, que lleva todas las filas de un proyecto en una. Esa transacción vuelve a comprobar cada fila. Una que cambió entre el cálculo y la escritura queda en `kept`, y una que no tiene vector no vuelve.
- **Las filas vuelven como estaban.** `undelete` sube `update_count` y `last_modified_at` porque es una edición de la persona (`RESTORE_ROW`, `admin.rs:565`). El `--undo` deshace, igual que el de una unión o el de un movimiento, así que no los toca.
- **Las negativas son las de `undelete`.** Lo eligió la persona: `live_twin`, `topic_taken` y `holds_secret`. Además, `changed` para una fila que ya no está como la dejó N, y `tied` para que las piezas de un save queden juntas: son las mismas reglas de unidad que usa el `--undo` de un import.
- **No corre el detector**, igual que el import, para no hacer una búsqueda KNN por fila. Los pares de la fila nunca se borraron, y los escalados vuelven con `settle_challengers`.
- **`deleted_later`.** Si X se borró con N, volvió al usarse y se borró otra vez con M, las filas de N que siguen borradas están en el conjunto de M como «ya borradas». Deshacer N primero restauraría filas bajo un proyecto que M tiene borrado. Así que M se deshace primero, como `merged_later` en D16.
- **El modelo sólo hace falta si hay filas que restaurar.** Las negativas son SQL y `redact`, así que una prueba en seco y el `--undo` de un proyecto vacío no lo necesitan.

| Alternativa | Por qué no |
|---|---|
| `restore_memory_core` fila por fila | Es un embedding y una transacción por fila, sube `update_count` y corre el detector por fila. Y sólo sabe restaurar `memory` |
| Devolver como `superseded` lo que quedaría borrado | La persona eligió lo de `undelete`. Además, hoy no hay código que restaure una fila como reemplazada |

## D34. La purga de un borrado saca todo su conjunto

Lo eligió la persona: «sacar de la memoria todo lo de un proyecto» incluye lo que ya estaba borrado antes de N. La purga va por las filas de `project_delete_row`, estén donde estén ahora. Si alguien movió una fila borrada a otro proyecto, sigue siendo del conjunto que la persona confirmó.

Vive en `crates/clawg-core/src/memory/secrets.rs`, al lado de `purge`, para compartir `open_to_rewrite` (`:309`), `leave_no_old_copy` (`:326`) y la lectura de snapshots. En lugar de repetir `purge` por fila, hace:

- **una transacción** con todas las filas;
- **un solo `optimize`, un solo `VACUUM` y un solo vaciado del WAL**, al final;
- **una sola pasada por los backups**: cada snapshot se abre una vez y se busca cualquiera de los ids con su texto. `backups_holding` (`:377`) mira un id por snapshot;
- **una lectura por cada `workspace.md`**, buscando cualquiera de los textos (`workspaces_holding`, `crates/clawg-core/src/memory/workspace.rs:602`). Los del proyecto se nombran siempre, como pedía la propuesta.

En `memory` hace con cada fila lo que hace `purge` con una fila borrada. No revive nada, porque las filas ya están borradas.

| Alternativa | Por qué no |
|---|---|
| Sólo las filas que borró N, como decía la propuesta | Las que ya estaban borradas quedarían, y habría que purgarlas de a una con `clawg purge <ID>` |
| Llamar a `purge` por cada fila | Un `VACUUM` y una pasada por todos los backups por fila |
| Reusar `remove_in` del `--undo` de un import (`portability/undo.rs:710`) | Borra también los enlaces en los que la fila es fuente, y así la conclusión no sabría que perdió una. Tampoco hace `VACUUM` |

## D35. Lo que administra proyectos no toma uno borrado

Es el criterio de D23 aplicado a un proyecto borrado: la persona nombró un proyecto que no existe para ella.

- **`rename`**, **`keep-apart`** y **los dos lados de una unión** responden 409 `project_deleted`. Unir un proyecto borrado dejaría sus filas borradas bajo Z, fuera del alcance de su `--undo`.
- **El `--undo` de una unión cuyo destino está borrado** también se rechaza. Si no, devolvería al origen filas que el borrado anotó bajo Z, y el `--undo` del borrado ya no las encontraría donde las dejó.
- **`to_project` de un movimiento y `project_id` de un `PATCH`** se rechazan en `target_in` (`crates/clawg-core/src/memory/relocate.rs:438`), que ya corre en la transacción que mueve.
- **`from_project` sí acepta un proyecto borrado**: rescatar una memoria borrada antes de purgar es un uso legítimo.
- **El `--undo` de un movimiento que le devuelve filas** no se rechaza: esas filas eran suyas, y el disparador lo revive (D27).

## D36. Los candidatos para vaciarlo

La regla de pares de D25 (mismo nombre, o mismo primer commit sin un remoto que los distinga, salteando los pares respondidos) vive hoy en la CLI, sobre los hechos de `inspect` (`project_duplicates`, `crates/clawg-cli/src/doctor/checks.rs:654`). Ahora el daemon la necesita para responder `not_empty`. Por eso pasa al core como una función pura sobre esos mismos hechos. `doctor` la llama, y el borrado le pasa los hechos que lee en su transacción. Así la regla queda en un solo lugar, y los candidatos de un borrado son los duplicados que nombraría `doctor`, más las sugerencias pendientes de X.

## D37. `doctor`

`inspect` suma dos hechos, leídos sólo donde el esquema los tiene (`Has`):

- `deleted_at` en `RegistryRow`;
- los conteos de filas vivas en `IdRows`, al lado de los de todas.

`project_duplicates` saltea los borrados. `project_orphans` y `project_merges` no cambian: un proyecto borrado está en el registro y no tiene `merged_into`. `project_deletes` es un chequeo nuevo, sólo con `live_under_deleted`.

## Costos en el camino caliente de la entrega 2b

- Cada insert de una fila viva con proyecto suma, en su disparador, un `UPDATE` por clave primaria que no encuentra nada. Lo mismo cada cambio de proyecto y cada restauración.
- Un borrado lee y escribe en una transacción las filas de X y su diario. No necesita el modelo.
- Un `--undo` embebe sus filas, a lo que tarde el modelo por tanda, antes de su transacción.
- Una purga hace un `VACUUM` del archivo entero, como `purge`, con el daemon parado.

## Volver atrás de la entrega 2b

- **El esquema.** Un binario anterior rechaza un store en V29 sin tocarlo (`SchemaNewer`), y la vuelta atrás es su snapshot `pre-migration`. Para quitar V29 a mano, primero hay que deshacer cada borrado abierto (`clawg projects delete --undo N`). Si no, un proyecto borrado vuelve a listarse, con sus filas borradas y su `index_docs` en 0. Un borrado purgado ya no se deshace: su proyecto vuelve a listarse vacío. Después, con un `sqlite3` 3.35 o más nuevo y el daemon parado:

  ```sql
  DROP TRIGGER memory_project_restored;
  DROP TRIGGER memory_project_ai;
  DROP TRIGGER memory_project_au;
  -- volver a crear los dos disparadores de V27 con su texto (V27__project_registry.sql:86-104)
  DROP TABLE project_delete_folder;
  DROP TABLE project_delete_row;
  DROP TABLE project_delete;
  ALTER TABLE project DROP COLUMN deleted_at;
  DELETE FROM refinery_schema_history WHERE version = 29;
  ```
- **Un borrado.** Con `clawg projects delete --undo N`, o con su snapshot `pre-delete`.
- **Una purga.** Sólo con un snapshot, que guarda justo el texto que se quería sacar.

# Entrega 3a: proyectos sin carpeta y el agente

Esta parte resuelve la **entrega 3a** de la [propuesta](proposal.md#3-proyectos-sin-carpeta-y-el-agente): el argumento `project` del contrato, crear un proyecto sin carpeta, los nombres donde hoy van ids, `projects create|locate`, las sugerencias que el agente ve y responde, y la sugerencia por nombre de carpeta. Los requisitos están en la [spec](spec.md#entrega-3a-proyectos-sin-carpeta-y-el-agente) (PL32 a PL42).

La persona eligió cuatro cosas el 2026-10-26:

- partir la entrega 3: los subproyectos de un monorepo van en una 3b con su propia migración (D49);
- que el agente responda una sugerencia con una herramienta nueva, que pide la respuesta de la persona para unir y para separar (D45);
- que el id de un proyecto borrado lo haga volver con un save, y que su nombre lo muestre como candidato (D42);
- que el agente cree un proyecto aparte, aunque haya parecidos, con `new_project` (D41).

## D38. Una referencia se resuelve en el daemon, en una ruta propia

El puente no tiene el registro, y los hooks, la CLI y el puente tienen que resolver igual. Por eso `POST /v1/projects/resolve` resuelve y, si se pide, crea, en una transacción. El orden es el que fijó D8: id, después nombre. El id gana porque es único y porque un nombre como `release-2024` puede tener la forma de un id sin serlo.

| Alternativa | Por qué no |
|---|---|
| Resolver en cada ruta que toma un proyecto (`/v1/memories`, `/v1/search`, …) | Cada ruta tendría que responder candidatos en su propia forma, y un save rechazado por un parecido ya habría pasado la validación del outbox. Con una ruta aparte, el save recibe un id, como hoy |
| Resolver en el puente leyendo `GET /v1/projects` | Lee carpetas en disco por cada llamada, y crear quedaría fuera de la transacción que mira el nombre |

## D39. Parecido es la misma clave de nombre

La clave (`name_key`) es el nombre en minúsculas, sin nada que no sea letra o dígito. Junta lo que la gente escribe distinto para lo mismo (`Mi App`, `mi-app`, `MiApp`), y nada más.

| Alternativa | Por qué no |
|---|---|
| Distancia de edición | `api` y `app`, o `web` y `wiki`, serían parecidos: el agente preguntaría por proyectos que no tienen nada que ver |
| Sólo el nombre exacto, como `project_duplicates` (D25) | `Mi App` y `mi-app` crearían dos proyectos sin preguntar |
| Comparar también con el nombre de cada carpeta | La carpeta ya da el nombre del proyecto que nace de ella (D6). Una carpeta renombrada a mano es justo lo que el usuario cambió |

Los borrados entran como candidatos, marcados, y las lápidas no: el nombre de una lápida quedó libre (PL15) y su id lleva a otro proyecto, que ya está entre los vivos.

## D40. El id de un proyecto nuevo

`slug-4hex` (decisión 6 de la propuesta). El slug sigue la forma de los ids de hoy (`derive_project_id`), que nacen de un nombre de carpeta: minúsculas ASCII, dígitos y `-`. Un nombre sin letras ASCII (`проект`) da `project-XXXX`. El nombre conserva lo que escribió la persona.

Los cuatro hex salen de SHA-256 sobre el nombre, la hora en milisegundos y un contador, y se prueban hasta 64 contadores contra `project` y `project_path`. No se derivan sólo del nombre: el nombre de un proyecto borrado queda libre (PL27), y un proyecto nuevo con ese nombre no puede tomar el id del borrado.

## D41. Crear aparte, con `new_project`

Cuando hay parecidos, el agente le pregunta a la persona. Si la persona dice que es otro proyecto, el agente vuelve a guardar con `new_project: true`. El nombre exacto de un proyecto vivo nunca crea otro: con `new_project` también resuelve a ese proyecto, porque dos proyectos vivos con el mismo nombre elegido son lo que D8 evita.

`new_project` no deja una sugerencia con los parecidos: la persona ya respondió.

| Alternativa | Por qué no |
|---|---|
| Sólo por CLI (`clawg projects create --new`) | La persona tendría que salir de la conversación para algo que ya contestó |

## D42. Un proyecto borrado en el argumento `project`

- **Por id:** se resuelve a sí mismo, y un save bajo él lo hace volver por el disparador de D27, como un save desde un puente abierto. Buscar bajo él no encuentra filas vivas, y eso está bien.
- **Por nombre:** su nombre quedó libre, así que no resuelve. Aparece como candidato `deleted` con su borrado abierto, y el agente pregunta: volver a usarlo es guardar con su id; uno nuevo, `new_project`.

## D43. Las respuestas que leen candidatos son 200

Como en D31: una respuesta con candidatos es lo que el agente necesita para preguntar, y `ErrorBody` no tiene lugar para una lista. `refused` vale `ambiguous`, `similar` o `unknown`. Los 400 quedan para una referencia vacía o demasiado larga.

## D44. El argumento en el puente, y las búsquedas sin proyecto

`project` y `project_path` se excluyen: los dos dicen dónde. `project` se resuelve en cada llamada; el puente no lo guarda, porque un nombre puede pasar a otro proyecto con un `rename`, y la ruta cuesta una lectura por clave.

Sólo `save_memory` crea, porque un save es lo único que necesita un proyecto donde quedar. Una búsqueda con un nombre que no existe no tiene nada que encontrar, y crear ahí dejaría proyectos vacíos. `update_memory` mueve a un proyecto que ya existe: para uno nuevo, primero se guarda en él o se crea con `clawg projects create`.

`search_project` con `project` busca en la carpeta más vieja con `index_docs`, o en la más vieja si ninguna lo tiene (y entonces el daemon responde su 403 con el comando que la agrega). Un proyecto sin carpeta no tiene archivos, y lo dice.

**Una búsqueda cuando el directorio del puente no da ningún proyecto** sigue sin alcance, como hoy (la decisión abierta de la propuesta). El caso es un puente sin directorio de trabajo, y ahí un error no ayudaría: `project` ya da la forma de nombrar uno.

## D45. El agente responde una sugerencia con `answer_suggestion`

Lo eligió la persona: una herramienta nueva, con `answer` (`merge` o `apart`), que pide `user_answered` y la respuesta en `reason` para los dos, como `resolve_conflict` con una preferencia. Unir mueve todo lo de un proyecto, y separar hace que el par no se vuelva a preguntar: las dos son decisiones de la persona.

La ruta sólo responde una sugerencia pendiente (404 `no_suggestion` si no la hay): el agente no une proyectos que nadie sugirió. Unir pasa `other` a `project`, el del puente por defecto, con `merge_core` (D18 a D21), así que tiene su snapshot, su diario y su `--undo`. Como lo que administra proyectos (D23, D35), no sigue lápidas ni toma borrados: el contexto le muestra al agente los ids vivos.

`reason` no se guarda: `project_suggestion` no tiene columna para eso, y la respuesta sólo dice unir o separar. Pedirla hace que el agente tenga que preguntar.

| Alternativa | Por qué no |
|---|---|
| Un argumento en una herramienta existente (`update_memory`, `resolve_conflict`) | Ninguna trata de proyectos, y su descripción se volvería confusa |
| Sólo separar desde el agente, unir por CLI | Unir se deshace con `--undo`, y es la respuesta más común a una copia o a una mudanza |

## D46. La sugerencia por nombre de carpeta

La decisión 3 de la propuesta: una carpeta con el nombre de un proyecto cuyo camino ya no existe es evidencia débil. Se aplica a una raíz que, después de PL4 y PL5, abre un proyecto nuevo: es la carpeta sin VCS (o con VCS sin primer commit) que se movió. La condición es que **todos** los checkouts grabados del otro proyecto ya no estén: si alguno sigue, es otro proyecto con el mismo nombre (dos carpetas `api`), y preguntar cansaría.

El disco se mira en la misma pasada que las mudanzas (`Planned::Presence`, D9), sumando los checkouts de esos proyectos. Las sugerencias son las de PL6: una por par, que una respuesta guarda.

## D47. Nombres en `--to-project` y en `PATCH`

El daemon resuelve el nombre al entrar, antes de `target_in`, y no busca parecidos ni crea: quien mueve a mano nombra un proyecto que existe. El id va primero y sin seguir la lápida, porque `target_in` la sigue (o la rechaza) en la transacción, como hoy.

## D48. `projects locate` une el proyecto de la carpeta en el nombrado

«Esta carpeta es el proyecto X» quiere decir que lo guardado desde ella es de X. Cuando la carpeta ya tiene su proyecto P, con su única carpeta, eso es unir P en X: la unión lleva sus filas, su carpeta y sus remotos, y deja su diario y su `--undo`. Si P tiene otras carpetas, unirlo se llevaría también esas, que nadie nombró: `folder_shared` lo dice, y la persona decide con `projects merge`.

| Alternativa | Por qué no |
|---|---|
| Reescribir `project_path.project_id` de la carpeta | Las filas guardadas desde ella quedarían en P, y no tendría `--undo` |
| Rechazar si P tiene filas | Es el caso común: una carpeta movida que ya recibió saves antes de ubicarla |

## D49. Lo que queda para la entrega 3b

Los subproyectos de un monorepo cambian la resolución de una raíz (la carpeta registrada más profunda, con el camino crudo del cliente) y la clave de `project_remote` (remoto más subcarpeta), que es su clave primaria (`V24__project_remote.sql:9`). Eso pide reconstruir la tabla o guardar la subcarpeta aparte, y cambia `POST /v1/projects`, los hooks y el import. Es un riesgo distinto del de esta entrega, que no toca el esquema ni la resolución de raíces, así que va en la 3b, con su spec, su diseño y su migración.

## Costos en el camino caliente de la entrega 3a

- Una herramienta con `project` suma un pedido a `POST /v1/projects/resolve`: una lectura por clave, o una pasada por los nombres de `project` (decenas de filas).
- Cada `context` suma la lectura de las sugerencias pendientes, por el índice del par.
- Un registro de un camino nuevo que abre un proyecto suma la lectura de los nombres y, si alguno coincide, un `stat` por cada carpeta de ese proyecto.

## Volver atrás de la entrega 3a

- **El esquema.** No cambia.
- **Un proyecto creado por nombre.** Es un proyecto como otro: se borra con `clawg projects delete` (vacío, o con sus memorias) o se une con `merge`.
- **Una unión de `answer_suggestion` o de `locate`.** Con `clawg projects merge --undo N`.
- **Una respuesta `apart`.** Como la de `keep-apart`: el par queda respondido. Para volver a preguntarlo, se borra su fila de `project_suggestion` a mano.
