# project-locators — Diseño

Este diseño resuelve la **entrega 1** de la [propuesta](proposal.md#1-el-registro-v27) y las decisiones de [«Abierto para el diseño»](proposal.md#abierto-para-el-diseño) que le tocan:

- el localizador `path` frente a R20 (D1);
- `project_remote` (D2);
- cómo leer el primer commit (D3);
- la integridad referencial (D7);
- el nombre (D8).

Las demás esperan a su entrega (al final). Los requisitos están en la [spec](spec.md).

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
| Una búsqueda cuando el cwd no resuelve a ningún proyecto | 3 |
| Cómo responde el agente una sugerencia, y si unir exige `user_answered` | 3 (la respuesta por CLI es de la 2) |
| Un save bajo el id de un proyecto borrado | 2 |
| Los tipos nuevos de `memory_import_extra` | 4 |
| Los subproyectos de un monorepo y las carpetas sin VCS | 3 (D14) |
