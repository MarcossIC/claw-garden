# project-locators — Diseño

Este diseño resuelve la **entrega 1** de la [propuesta](proposal.md#1-el-registro-v27) y las decisiones de [«Abierto para el diseño»](proposal.md#abierto-para-el-diseño) que le tocan:

- el localizador `path` frente a R20 (D1);
- `project_remote` (D2);
- cómo leer el primer commit (D3);
- la integridad referencial (D7);
- el nombre (D8).

Las demás esperan a su entrega (al final). Los requisitos están en la [spec](spec.md). La [entrega 2a](#entrega-2a-unir-proyectos) (D15 a D25), la [2b](#entrega-2b-borrar-proyectos) (D26 a D37), la [3a](#entrega-3a-proyectos-sin-carpeta-y-el-agente) (D38 a D49), la [3b](#entrega-3b-los-subproyectos-de-un-monorepo) (D50 a D59) y la [4](#entrega-4-exportar-e-importar) (D60 a D70) están después de las decisiones de la entrega 1.

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

Lo que cambia es el remoto **de cada carpeta** (`project_path.remote`). Hasta R20, los remotos de una carpeta eran los que `project_remote` ataba a su id. Por eso una carpeta que entró al proyecto de otra por su remoto no tenía remotos propios y nunca se contradecía, y el [diseño de 89](../2026-10-25-project-registry/design.md#d1-la-huella-valida-la-fila-en-una-función-pura) aceptó ese costo. Desde V27 cada carpeta graba el suyo. Una carpeta de antes con un único remoto atado lo recibe en el relleno. Una fundida (dos o más) queda en NULL y se sigue eligiendo por sus remotos atados, como en R20: no se adivina cuál es el suyo.

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
| Volver a importar un export de clawg después de un movimiento simple | 4 ([D65](#d65-importar-otra-vez-después-de-un-movimiento-la-fila-está-en-cualquier-proyecto)) |
| Una búsqueda cuando el cwd no resuelve a ningún proyecto | 3a ([D44](#d44-el-argumento-en-el-puente-y-las-búsquedas-sin-proyecto)) |
| Cómo responde el agente una sugerencia, y si unir exige `user_answered` | 3a ([D45](#d45-el-agente-responde-una-sugerencia-con-answer_suggestion)); la respuesta por CLI es de la 2a, D22 |
| Un save bajo el id de un proyecto borrado | 2b ([D27](#d27-un-proyecto-borrado-vuelve-al-usarse-por-disparadores)) |
| Los tipos nuevos de `memory_import_extra` | 4: una tabla nueva, `memory_import_registry` ([D69](#d69-v31-una-tabla-de-diario-nueva-y-un-índice-por-fecha)) |
| Los subproyectos de un monorepo y las carpetas sin VCS | las carpetas sin VCS, 3a (D46, D48); los subproyectos, 3b ([D50 a D59](#entrega-3b-los-subproyectos-de-un-monorepo)) |

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

**Dónde se ve.** `recall_context` da una línea por sugerencia, con su motivo. `session-start` da una sola, que nombra hasta dos proyectos y manda a `recall_context` por el motivo: el hook tenía un tope de 1.000 bytes para lo que suma, y el peor caso de antes (la compactación, el modelo que no cargó con su causa, los dos avisos de síntesis) ya ocupaba 978. El tope pasa a 1.500, que con los 8.000 del contexto (`SESSION_BUDGET`) sigue entrando en los 10.000 caracteres que inyecta el host.

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

# Entrega 3b: los subproyectos de un monorepo

Esta parte resuelve la **entrega 3b** de la [propuesta](proposal.md#3-proyectos-sin-carpeta-y-el-agente): declarar una subcarpeta de un repositorio como un proyecto propio, el camino crudo que manda el cliente, la resolución por la declaración más profunda y su migración. Los requisitos están en la [spec](spec.md#entrega-3b-los-subproyectos-de-un-monorepo) (PL43 a PL52).

D14 y D49 dejaron escrito cómo sería esta entrega. Por lo que muestra el código, eso cambia en tres puntos, y cada uno tiene su decisión:

- **La clave no es el remoto.** D14 pedía remoto más subcarpeta, lo que obligaba a reconstruir `project_remote`, cuya clave primaria es `remote` (`V24__project_remote.sql:9`), o a guardar la subcarpeta aparte. Se guarda aparte, pero con el proyecto del repositorio en lugar del remoto, y `project_remote` no cambia (D51).
- **Sólo cuentan las subcarpetas declaradas.** Leída sobre todas las carpetas de `project_path`, «la carpeta registrada más profunda» de D14 metería en el proyecto del directorio del usuario cada carpeta sin git que haya debajo (D52).
- **`project_path` sigue guardando raíces.** Una declaración no deja una carpeta por checkout, así que ningún lector de `project_path` cambia (D52).

## D50. Una subcarpeta se declara a mano, con `projects locate --subfolder`

El registro automático no puede adivinar un subproyecto (D14). Un monorepo tiene un manifiesto por paquete (`apps/web/package.json`, `libs/ui/package.json`), y casi ninguno es un proyecto aparte para la persona. Por eso se declara, una vez por subproyecto, con `clawg projects locate <PROYECTO> <CAMINO> --subfolder`.

3a fijó `locate` como «esta carpeta es el proyecto X» (D48). Sin la marca, `locate` sube de CAMINO a su raíz (`locate`, `crates/clawg-cli/src/projects.rs:898-905`). Con ella, la carpeta es CAMINO mismo. La ruta es la misma, `POST /v1/projects/locate`, con `subfolder: true`. Su cuerpo rechaza los campos que no conoce (`#[serde(deny_unknown_fields)]`, `crates/clawg-cli/src/daemon/api.rs:2435`), así que un daemon sin subcarpetas responde 400 en lugar de ubicar la raíz entera, que uniría el proyecto del monorepo.

PROYECTO tiene que existir. Uno nuevo se crea antes con `clawg projects create`, que busca parecidos (PL39).

**El agente no declara.** Una declaración cambia adónde va lo que guarda cualquier agente desde esa carpeta, en cada sesión y en cada clone, y se hace una vez. El agente ya puede guardar en un proyecto por nombre (`project`, PL34). El contrato del modelo no cambia.

| Alternativa | Por qué no |
|---|---|
| Detectar los subproyectos solos, por manifiesto | Cada paquete tiene el suyo, y casi ninguno es un proyecto aparte. Una sesión en un paquete leería sólo sus memorias, sin que nadie lo haya pedido |
| Un comando propio (`clawg projects subfolder`) | Es otra palabra para «esta carpeta es X», que ya es `locate` |
| Una herramienta del agente | Cambia el contrato, y necesitaría la respuesta de la persona, como `answer_suggestion` (D45), para algo que se hace una vez |
| Que `--subfolder` cree el proyecto si no existe | Un nombre mal escrito crearía un proyecto vacío sin preguntar |

## D51. La declaración es del proyecto del repositorio, no del remoto

Una declaración dice: en el proyecto P de un repositorio, lo que se trabaja dentro del subcamino S va al proyecto X. Se guarda en una tabla nueva, `project_subfolder(parent_id, subpath, project_id, created_at)`, con la clave primaria `(parent_id, subpath)`. `parent_id` es P, no un remoto.

D14 pedía el remoto más la subcarpeta para que un clone en otra máquina llegara al subproyecto. Pero un clone ya llega a P por su remoto (R16; `remote_project_in`, `crates/clawg-core/src/memory/project.rs:247-249`). Con la declaración atada a P, llega también a X, sin un segundo mecanismo. Lo mismo vale para todo lo que ya lleva a P:

- un checkout sin `origin` que se movió y conserva su proyecto por el primer commit (PL5). Con el remoto como clave, no tendría subproyectos;
- un worktree, cuya raíz es la del repositorio principal (`root_from_marker`, `crates/clawg-core/src/memory/identity.rs:118-127`);
- el store abierto en otra máquina (un `clawg restore`, o el directorio de datos copiado). La declaración no guarda caminos de esta máquina, así que sirve igual allí;
- un repositorio cuyo remoto cambió y se unió con su proyecto viejo (PL6 y `projects merge`): la declaración se lee a través de la lápida (D54). Con el remoto como clave, el remoto nuevo no la encontraría nunca.

`project_remote` no cambia. Su clave primaria es `remote` (`V24__project_remote.sql:9`), que no admite el mismo remoto dos veces. Sumarle una columna `subpath` no alcanza, y cambiar la clave obliga a reconstruir la tabla. V30 sólo crea una tabla, como promete la propuesta en [«Datos guardados»](proposal.md#datos-guardados-migración-y-vuelta-atrás).

| Alternativa | Por qué no |
|---|---|
| Reconstruir `project_remote` con la clave `(remote, subpath)` | No es aditiva. Además, cada `WHERE remote = ?1` de hoy pasaría a encontrar dos filas y tendría que sumar `subpath = ''`: `remote_project_in` (`project.rs:248`), el `--undo` de un import (`crates/clawg-core/src/memory/portability/undo.rs:694`) y el de una unión (`crates/clawg-core/src/memory/merge.rs:514`). Uno que se olvide elige mal, sin error |
| Una tabla aparte con la clave `(remote, subpath)`, como pedía D14 | Es aditiva, pero un repositorio sin `origin` no tendría subproyectos, uno movido sin `origin` los perdería, y uno con el remoto renombrado también. Esos casos necesitarían otra regla, por carpeta: dos mecanismos para lo mismo |
| El subcamino dentro del texto del remoto (`github.com/acme/mono//apps/web`) | No cambia el esquema, pero cada lector de remotos (`projects list`, `doctor`, el código de un borrado) tendría que partir el texto |

Lo que cuesta: una declaración vale para todo lo que lleva a P, también para otro repositorio que la persona unió en P (D18). Eso es lo que dijo al unirlos: que son uno.

## D52. Sólo cuentan las subcarpetas declaradas, y `project_path` sigue guardando raíces

D14 decía que la resolución elegiría «la carpeta registrada más profunda que contiene al camino», y que una carpeta de `project_path` podría ser cualquier directorio. Leído sobre todas las carpetas, eso cambia lo de hoy:

- **El directorio del usuario.** Fuera de git, la raíz de un camino es el camino mismo (`resolve_project_root`, `identity.rs:72-84`). Los hooks y el puente registran la raíz de su directorio sin filtrarla (`project_of`, `crates/clawg-cli/src/hook.rs:221-228`; `ids_for`, `crates/clawg-cli/src/mcp/server.rs:915-952`), así que una sesión abierta en el directorio del usuario lo deja registrado. Con la carpeta más profunda, cada carpeta sin git que haya debajo caería en ese proyecto.
- **Las carpetas viejas de un camino.** Dentro de un repositorio, una carpeta de `project_path` en un camino que hoy tiene otro repositorio (R20) llevaría a un proyecto que las huellas ya contradicen.

Por eso sólo cuentan las declaraciones, y sólo dentro del checkout del camino: una declaración nunca sube por encima de su cima.

Una declaración tampoco deja una carpeta en `project_path` por cada checkout. Así, ningún lector de `project_path` tiene que distinguir una raíz de una subcarpeta:

- `path_folders_in` (`project.rs:233-239`);
- `sightings_in` (`:330-333`), donde una subcarpeta con el primer commit del repositorio parecería un checkout movido de otro proyecto;
- `named_after_in` (`:365-367`);
- `folders_of_in` (`crates/clawg-core/src/memory/registry.rs:756-762`);
- `set_in` (`crates/clawg-core/src/memory/project_delete.rs:330-350`);
- el índice de docs (`root_of`, `crates/clawg-core/src/memory/project_docs/mod.rs:505`).

| Alternativa | Por qué no |
|---|---|
| Que cuente toda carpeta registrada, como decía D14 | El directorio del usuario y las carpetas viejas de un camino, arriba |
| Una carpeta de `project_path` por subcarpeta y checkout, marcada con su padre | Cada lector de arriba tendría que saltearla. `index_docs` vale 1 por defecto (`V13__project_index_docs.sql:6`), así que un insert que olvide la columna indexaría la subcarpeta. Y si su id es el de un proyecto con filas, el export la escribiría como una línea `project` (`crates/clawg-core/src/memory/portability/export.rs:136-144`), que el import registraría como una raíz |

**El índice de docs** sigue por carpeta de raíz, como fija la propuesta en «Fuera de alcance». Un subproyecto no tiene índice propio: sus docs son los de su repositorio (D59).

## D53. El subcamino se mide desde la cima del checkout, y lo calcula el daemon

El cliente manda en `POST /v1/projects`, además de `root_path`, el camino crudo: `path`, absoluto. En el mismo hilo bloqueante en el que hoy lee el checkout (`register_root`, `crates/clawg-cli/src/daemon/api.rs:2113-2116`), el daemon lee la cima del checkout de `path` (el directorio con su `.git`) y comprueba que la raíz de esa cima sea `root_path`. El subcamino es `path` relativo a esa cima. Si la raíz es otra, o `path` no está en un checkout, el daemon ignora `path`, lo deja en el log y responde como sin él.

**Desde la cima, no desde la raíz.** En un worktree, la raíz es la del repositorio principal (R1; `root_from_marker`, `identity.rs:118-127`), y el camino crudo no está debajo de ella: `/work/mono-wt/apps/web/src` tiene la raíz `/work/mono`. Medido desde la cima, `/work/mono-wt`, da `apps/web/src`, el mismo subcamino que en el checkout principal.

**El daemon, no el cliente.** El daemon decide los ids con lo que lee él (R4, D3). Además, un `project_path` relativo se resuelve contra el directorio del puente, que no es el del daemon. Por eso el cliente lo manda absoluto, en la forma canónica desde la que sube `resolve_project_root`. `ProjectIdentity` guarda ese camino (`path`) al lado de `root` y `legacy_id` (`identity.rs:45-57`).

La parte que decide es pura. `subpath_of(cima, camino)` une con `/` los componentes de `camino` que están debajo de `cima`, y no da nada si el camino sale de la cima o si un componente no es Unicode válido. La lectura del disco entra por `Ground::subpath(root, path)`, al lado de `checkout` (`identity.rs:338-346`), así que `FakeGround` la responde en los tests y `ReadAhead` en lo que planea sin escribir.

**Los anteriores.**

- Un cliente que no manda `path` recibe el proyecto del repositorio, como hoy.
- Un daemon anterior ignora el campo y responde el proyecto de la raíz. En v0.63.0, `RegisterRequest` no rechaza campos que no conoce (`git show v0.63.0:crates/clawg-cli/src/daemon/api.rs`, líneas 1986-1987).
- Un pedido sin `assigned` ignora `path`, porque deriva sus ids de la raíz (R18).

**Qué manda cada uno:**

| Quién | `path` |
|---|---|
| `session-start` y `user-prompt-submit` | el `cwd` de la entrada |
| el puente, para su proyecto por defecto | su directorio de trabajo al arrancar (`ClawgMcp::pending`, `server.rs:528-533`) |
| el puente, con `project_path` o `to_project_path` | ese camino |
| `projects move --to CAMINO` y `--here` | CAMINO, y el directorio de trabajo |
| `memories --here` y `export --project CAMINO` | el directorio de trabajo y CAMINO, a `project_id_of`, que lee sin escribir |
| `projects add` | nada: agrega la raíz y su índice de docs |
| `projects locate` sin `--subfolder` | nada: ubica la raíz, como en 3a |
| el import | nada (D59) |

`project_id_of` abre stores de versiones anteriores sin migrarlos. A uno anterior a V30 le presta una tabla temporal vacía, como hace con `project_remote` (`export.rs:100-102`).

El puente registra una vez por raíz y camino, y no por raíz e id viejo (`registered`, `server.rs:491`, con la clave de `ids_for`, `:929`). Dos caminos con el mismo nombre de carpeta pueden dar el mismo id viejo, porque el hash tiene cuatro dígitos (`derive_project_id`, `identity.rs:16-20`). `save_project` (`server.rs:968-973`), que dice dónde revisa una clave, busca la entrada por raíz y camino.

| Alternativa | Por qué no |
|---|---|
| Que el cliente mande el subcamino | El daemon dejaría de decidir con lo que lee, y cada cliente repetiría la cuenta del worktree |
| Medir desde la raíz | Un worktree no llegaría nunca a sus subproyectos |
| Responder 400 a un `path` que no lleva a su raíz | Los hooks no pueden fallar por eso, porque un 400 deja la sesión sin contexto. Ignorarlo da el proyecto del repositorio, que es lo de hoy |

## D54. Gana la declaración más profunda, leída por `canonical`

Con un subcamino S no vacío, después de asentar la raíz en el proyecto P (PL4, PL5), el registro hace esto:

1. Lee las declaraciones de P y las de las lápidas que llevan a P. Por D16, una lápida está a un salto, así que se encuentran por `idx_project_merged_into` (`V28__project_merge.sql:11-12`).
2. Se queda con las que contienen S por componentes enteros: `apps/web` contiene `apps/web/src`, pero no `apps/webapp`.
3. Elige la más profunda. A igual subcamino, la de P va antes que la de una lápida, y entre lápidas gana la declaración más vieja.
4. Responde el proyecto al que lleva la elegida, por `canonical`.

**Una unión no reescribe declaraciones.** Es la razón de D22 para las sugerencias: la clave primaria `(parent_id, subpath)` podría chocar con una declaración del destino, y el `--undo` tendría que volver a reescribirlas. Leídas por `canonical`:

- unir P en Z lleva sus declaraciones a Z;
- unir X en Z hace que lleven a Z;
- el `--undo` de cualquiera de las dos las devuelve solo, sin diario.

**Una declaración que lleva a P** (porque X se unió en P) responde P. Al unirlo, la persona dijo que esa subcarpeta es el repositorio. Si la unión se deshace, vuelve a llevar a X.

**Un subproyecto borrado** se responde a sí mismo, como cualquier proyecto borrado (D27): registrar no lo revive, y un save desde la subcarpeta sí.

**No se encadena.** Sólo se mira el proyecto del repositorio, nunca las declaraciones de X. Por eso una cadena no puede hacer un ciclo.

La decisión es pura: `deepest(declaraciones, S)` recibe las declaraciones ya leídas y el subcamino. Vive en un módulo nuevo, `crates/clawg-core/src/memory/subfolder.rs`, con la lectura y la escritura de las declaraciones.

| Alternativa | Por qué no |
|---|---|
| Reapuntar las declaraciones en la unión, con su diario | El choque de la clave y el `--undo`, como en D22. Además, `merge.rs` sumaría dos tipos a su diario para algo que la lectura ya resuelve |
| Que una declaración que lleva a P ceda ante una menos profunda | Una subcarpeta que la persona volvió a unir al repositorio terminaría en otro subproyecto |

## D55. Resolver una subcarpeta no une nada

El registro asienta la raíz como hoy, con sus uniones (R4, R17), y sólo después cambia el proyecto de la respuesta. Esto evita un riesgo concreto. El plan une en su proyecto los ids viejos y el id de la carpeta de la raíz cuando no son ese proyecto (`plan_in`, `project.rs:727-741`). Si el proyecto del plan fuera X, el id de la carpeta del monorepo entraría en esa lista, y como ese id suele ser el del proyecto del monorepo (D13), `merge_in` llevaría todo el monorepo a X.

Por eso `RegisterOutcome` conserva el proyecto de la raíz y suma `subproject`:

- los ids viejos de un camino crudo se unen en P, como hoy (R4). Son filas guardadas antes de que existiera la resolución de raíces;
- `adopt_legacy` copia los `workspace.md` viejos a P (`register_project`, `project.rs:951-957`);
- `root_id` sigue siendo el de la carpeta de la raíz, porque el índice de docs es suyo;
- la respuesta da `project_id` = X.

## D56. Lo guardado antes de declarar se queda donde estaba

Una fila de `memory` no guarda el camino desde el que se guardó: sólo su proyecto (`V1__init.sql:4-29`, más las columnas que sumaron V6, V16, V17, V22 y V23). Nada distingue lo que se guardó desde `apps/web` de lo que se guardó desde la raíz. Por eso declarar no mueve nada. La salida de `--subfolder` nombra el camino a mano: `clawg projects move --from-project P --to-project X --text PALABRAS --dry-run`, o con `--session ID` o `--id N` (R8).

| Alternativa | Por qué no |
|---|---|
| Mover por sesión (`source_session_id`) | Una sesión abierta en el monorepo trabaja en todas sus carpetas |
| Dejar una sugerencia entre P y X | Una sugerencia une proyectos, no filas: responderla con `merge` llevaría todo el monorepo a X |
| Mover las filas cuyo texto nombra la carpeta | Es adivinar fila por fila, y la propuesta lo deja fuera de alcance |

## D57. Declarar otra vez, quitar, y las sesiones abiertas

- **Otra vez, hacia otro proyecto.** Si S ya lleva a Q y la persona nombra X, vale lo de D48: lo guardado desde la subcarpeta es de X. Cuando lo único que lleva a Q es esa declaración (Q no tiene carpeta, ni remoto, ni otra declaración), se une Q en X con `merge_core`, con su snapshot, su diario y su `--undo`. La fila sigue nombrando a Q, y por `canonical` lleva a X. Si algo más lleva a Q, la respuesta es 409 `folder_shared` con la unión que haría falta, como en 3a.
- **Hacia P, o una que lleva a P.** Si la declaración de S lleva a P (D54), se la trata como si no hubiera ninguna: nombrar X la reemplaza, sin unir nada.
- **Quitar.** Nombrar el proyecto del repositorio (`clawg projects locate P CAMINO --subfolder`) borra la declaración de S, la de P y las de sus lápidas, y la carpeta vuelve a llevar a lo que lleva su padre. Lo guardado en X se queda en X. No hace falta diario: una declaración no mueve filas, y la vuelta atrás es declararla otra vez.
- **Las sesiones abiertas.** Un puente guarda los ids de cada raíz y camino toda la sesión (R6). Uno abierto antes de declarar sigue con P hasta que se reinicia. En cambio, los hooks registran en cada evento (`hook.rs:242`) y ya leen X. La salida de `--subfolder` lo dice.

| Alternativa | Por qué no |
|---|---|
| Un `--remove` aparte | Es otra forma de decir «esta carpeta es P» |
| Reapuntar la fila de Q a X sin unir | Lo guardado desde la subcarpeta quedaría en Q, contra lo que dice `locate` en D48 |
| Que el puente vuelva a registrar en cada herramienta | R6 lo evita a propósito: sería un pedido más por herramienta, para algo que pasa una vez |

## D58. Unir, borrar, la lápida y `doctor`

- **Unir.** No toca `project_subfolder` (D54), y el informe de una unión no cambia.
- **Lo que administra proyectos.** Declarar hacia una lápida responde 409 `project_merged`, y hacia un proyecto borrado, 409 `project_deleted` (D23, D35). El proyecto del repositorio sí puede estar borrado: la declaración se escribe bajo él, porque el registro sigue llevando a él (D27).
- **Borrar.** Un borrado no toca las declaraciones, como no desvincula carpetas ni remotos (D27). El informe (PL24) nombra las que llevan al proyecto, porque un save desde esas subcarpetas lo hace volver. El código (D29) suma una línea por cada una, después de los remotos, y sólo si las hay, así que el código de un proyecto sin declaraciones no cambia. Una purga tampoco las toca: no guardan texto.
- **`doctor`.** Un chequeo nuevo, `project_subfolders`, nombra con `warn` y `broken_subfolder` una declaración cuyo repositorio o destino no está en `project`. Sólo pasa en un store editado a mano. Una declaración bajo una lápida o hacia un proyecto borrado no es un hallazgo. `project_merges` cuenta filas, no declaraciones, así que no cambia. En un store anterior a V30, `inspect` no lee la tabla (`Has`, `crates/clawg-core/src/memory/inspect.rs:383`).
- **Las sugerencias.** Declarar no crea ninguna. La sugerencia por nombre de carpeta (D46) saltea un proyecto sin carpetas (`named_after_in`, `project.rs:372`), y un subproyecto puede no tener ninguna.

## D59. Lo que se ve, los docs y el import

- **`GET /v1/projects`.** Cada proyecto suma `subfolders`: las declaraciones que llevan a él y las de su repositorio, leídas por `canonical`. Cada una trae `repository_id`, `subpath`, `project_id` y sus caminos en esta máquina (cada carpeta del repositorio más el subcamino, con `present`). Los caminos se miran en el mismo hilo bloqueante que la presencia de las carpetas (`list_projects`, `registry.rs:211-230`). Un subproyecto sin carpeta propia y con declaraciones no se marca «sin carpeta».
- **Los docs.** El índice sigue por carpeta de raíz. Un puente en una subcarpeta recibe el `root_id` de su repositorio (D55), así que `search_project` busca en todo el monorepo. Con `project` (PL34), un proyecto sin carpetas propias y con declaraciones busca en las carpetas de sus repositorios: `POST /v1/projects/resolve` se las da en `folders` (`folders_of_in`, `registry.rs:756-762`).
- **La búsqueda de memorias** no cambia (`memory_project_predicate!`, `crates/clawg-core/src/memory/search.rs:107-114`). Desde un subproyecto se leen sus filas, las que no tienen proyecto y las `user_preference`, pero no las del repositorio. Leer también las del padre cambiaría ese predicado, y la propuesta lo deja fuera de alcance. `project` y `all_projects` llegan a ellas.
- **El import** no cambia. Una fila de un subproyecto viaja con su id y lo conserva aquí (`crates/clawg-core/src/memory/portability/import.rs:370`). `--project-map NOMBRE=CAMINO` sigue tomando la raíz de CAMINO (`crates/clawg-cli/src/import.rs:60-62`). La entrega 4 trae `project:ID|NOMBRE`, que puede nombrar un subproyecto, y decide si la línea `project` lleva las declaraciones.

## Costos en el camino caliente de la entrega 3b

- **Un registro con `path`** suma, en el hilo bloqueante que ya lee el checkout, un `canonicalize` y la subida hasta el `.git` del camino crudo: lo mismo que ya hizo el cliente. No corre `git`.
- **Con un subcamino no vacío** suma una lectura de `project_subfolder` por el prefijo de su clave, para P y sus lápidas, y un `canonical` de la elegida. Si el repositorio no tiene declaraciones, la lectura vuelve vacía.
- **No escribe nada.** Una declaración no deja una fila por checkout, así que un hook sigue sin escribir en cada evento.
- **El puente** registra una vez por raíz y camino, como antes lo hacía por raíz e id viejo.
- **`GET /v1/projects`** suma un `stat` por cada declaración y cada carpeta de su repositorio.

## Volver atrás de la entrega 3b

- **El esquema.** Un binario anterior rechaza un store en V30 sin tocarlo (`SchemaNewer`), y la vuelta atrás es su snapshot `pre-migration`. Una declaración no movió filas, así que antes no hay nada que deshacer. Para quitar V30 a mano, con el daemon parado:

  ```sql
  DROP TABLE project_subfolder;
  DELETE FROM refinery_schema_history WHERE version = 30;
  ```

  Desde ahí, lo que se guarde desde una subcarpeta va al proyecto del repositorio. Lo guardado en un subproyecto queda bajo su id: se alcanza con `project` o con `all_projects`, o se une con `projects merge`.
- **Una declaración.** Se quita nombrando el proyecto del repositorio (D57). Lo guardado en el subproyecto queda ahí, y `projects merge X --into P` lo devuelve, con su `--undo`.
- **Una declaración que unió un proyecto** (D57). Con `clawg projects merge --undo N`.

# Entrega 4: exportar e importar

Esta parte resuelve la **entrega 4** de la [propuesta](proposal.md#4-exportar-e-importar): la línea `project` con el nombre, los localizadores, la lápida y las declaraciones de un proyecto; el orden en que el import decide adónde va cada proyecto del archivo; `--project-map NOMBRE=project:REF`; las tres preguntas que la propuesta dejó abiertas, y el diario de lo que un import agrega al registro. Los requisitos están en la [spec](spec.md#entrega-4-exportar-e-importar) (PL53 a PL65). Depende de la 3b: V31 sigue a su V30, y el import usa su camino crudo.

Lo que muestra el código cambia la propuesta en tres puntos:

- **Hoy un proyecto puede viajar sin su línea.** El export escribe una línea por carpeta cuyo id es el proyecto de alguna fila exportada (`crates/clawg-core/src/memory/portability/export.rs:136-142`). Desde V27, el proyecto de un clone, el de un checkout movido, uno unido con otro y uno sin carpeta no tienen ninguna carpeta con su id. Sus filas viajan sólo con el id (`crates/clawg-core/src/memory/portability/import.rs:370`), y en el destino las adopta el disparador de PL2, con el nombre que da el id. Por eso la línea pasa a ser del proyecto y no de una carpeta (D60).
- **El import hace volver un proyecto borrado con una fila borrada.** Inserta viva cada fila borrada y después la marca, por el índice de texto (`import.rs:675-681`), y el disparador de inserción de V29 la ve viva (`crates/clawg-core/migrations/V29__project_delete.sql:63-64`). PL22 dice que insertar una fila ya borrada no hace volver a nadie. Se arregla en el import (D66).
- **Una sugerencia que el import no deja no aparece nunca.** PL5, PL6 y PL38 miran sólo un camino nuevo o contradicho (`plan_in`, `crates/clawg-core/src/memory/project.rs:656-703`). Una carpeta que grabó el import ya no es nueva para el próximo registro: la copia o el remoto cambiado que registra un import se quedan sin su sugerencia para siempre. Por eso el import pasa a dejarlas (D64).

## D60. Una línea `project` por proyecto, en la versión 1 del formato

```json
{"kind":"project","id":"app-1a2b","root_path":"/work/app","name":"App web","folders":[{"root_path":"/work/app","remote":"github.com/acme/app","root_commit":"c1…"},{"root_path":"/review/app","remote":"github.com/acme/app","root_commit":"c1…"}],"remotes":["github.com/acme/app"]}
{"kind":"project","id":"ideas-de-producto-3c4d","name":"Ideas de producto"}
{"kind":"project","id":"app-old-9f8e","name":"app-old","merged_into":"app-1a2b"}
{"kind":"project","id":"mono-5e6f","root_path":"/work/mono","name":"mono","folders":[…],"remotes":["github.com/acme/mono"],"subfolders":[{"subpath":"apps/web","project_id":"web-7a8b"}]}
```

- **`id` es el del proyecto.** Un lector de antes mapea las filas de ese id a lo que un registro le da a `root_path` (`prepare`, `import.rs:324-334`), así que lee mejor que hoy: hoy el proyecto de un clone no tenía línea.
- **`root_path` es la carpeta más vieja**, la que registra un lector de antes. Un proyecto sin carpeta no lo lleva.
- **Las huellas van por carpeta.** Un proyecto tiene varias carpetas y varios remotos (D2), y un camino puede tener varios repositorios (R20). `remotes` va aparte porque un remoto atado no siempre es el de una carpeta: el de una carpeta fundida de antes de V27 (D2) o el que entró con una unión.
- **Lo que no viaja.** `git_stamp` e `index_docs` son de esta máquina. `born`, `reviewed` y `created_at` cuentan lo que pasó en el store de origen: acá el proyecto lo crea un import, sin revisar (D63). `deleted_at`, en D61.
- **Lo vacío no se escribe** (`skip_serializing_if`). Así, el ejemplo fijado del formato (`crates/clawg-core/src/memory/portability/format.rs:301-314`) y la línea de un store anterior a V27 salen igual que hoy.

**La versión.** R1 de `specs/portability` pide subir `version` ante un cambio que un lector de la versión 1 leería mal. Un lector de antes (0.63.0 o anterior) ignora los campos nuevos (`unknown_fields_are_ignored`, `format.rs:354-360`). Una línea sin `root_path` no la lee mal: la reporta entre las líneas que no importó (`read_line`, `format.rs:224-237`) y deja las filas de ese proyecto con su id, que es lo que hacía con un proyecto sin línea. Subir a 2 haría que ese lector rechace el archivo entero (`FormatError::UnknownVersion`, `format.rs:181-182`).

| Alternativa | Por qué no |
|---|---|
| Subir el formato a la versión 2 | Un lector de antes rechaza el archivo entero, por no leer bien una línea que hoy ni siquiera existe |
| Seguir con una línea por carpeta, y un tipo de línea nuevo para lo demás | El proyecto queda repartido en dos tipos de línea, y un lector de antes reporta igual el tipo nuevo (`unknown kind`, `format.rs:227-229`) |
| Los proyectos en el encabezado | Un lector de antes los ignoraría sin reportar nada, pero el encabezado dice qué es el archivo (formato, versión, filtro), no el registro de un store con cientos de proyectos. Y el import lee las líneas, no el encabezado, para saber adónde va cada fila |
| Una sola huella por proyecto (`remote`, `root_commit`), como decía la propuesta | Un clone y una unión le dan a un proyecto varias carpetas y varios remotos (D2). Una sola huella obligaría a elegir una sin regla |

## D61. Qué proyectos llevan línea, y `deleted_at` no viaja

La línea de un proyecto sale cuando alguna fila exportada es suya, y además en tres casos sin filas:

- **Las lápidas que llevan a un proyecto exportado.** Un store que todavía tiene separados X y Z se entera de que la fuente los unió (D64). En un store sano no hay filas bajo una lápida (D17), así que su línea no lleva ninguna.
- **El repositorio de una declaración que lleva a un proyecto exportado.** La declaración es un dato del repositorio (D51), y sin su línea, `clawg export --project` desde `apps/web` la perdería.
- **El subproyecto de una declaración de un repositorio exportado.** Sin su línea, la declaración no tendría adónde llevar en el destino (D67).

**`deleted_at` no viaja** (la tercera pregunta de la propuesta). Un borrado es un acto de este store, con su diario: `project_delete`, su código, su snapshot y las filas de su conjunto (D28). Un archivo no puede traer ese diario. Marcar borrado un proyecto sin él dejaría un borrado que `deletes` no lista, sin `--undo` ni purga. Las filas sí traen su propio `deleted_at`. Con `--include-deleted`, el destino recibe un proyecto vivo que sólo tiene filas borradas, o sea vacío (PL23), y `clawg projects delete` lo borra al momento.

| Alternativa | Por qué no |
|---|---|
| Que `deleted_at` viaje y el import marque el proyecto borrado | Un borrado sin diario: `deletes` no lo lista, no tiene `--undo` ni purga, y `doctor` no lo puede nombrar |
| Que `deleted_at` viaje y el import escriba un borrado propio | Un borrado que nadie confirmó, con el código de un conjunto que nadie vio (D29) |
| Sólo las líneas de los proyectos con filas | No viajarían las lápidas ni las declaraciones de un monorepo |

## D62. Adónde va cada proyecto del archivo: el mapa, el remoto, el id, el nombre y el camino

Hoy el import decide sólo por el camino: registra en el plan la raíz de cada línea, o la de `--map-root` (`roots_here`, `import.rs:235-244`), y el plan lee el checkout de ese camino en este disco (`plan_ids_in`, `project.rs:549`). En otra máquina ese camino no existe, así que no hay remoto: un clone del mismo repositorio en otro camino abre un proyecto nuevo. Y en esta máquina, el mismo camino puede tener hoy otro repositorio (R20) y recibir las filas del de antes.

El orden (PL56), con lo que justifica cada paso:

1. **El mapa.** La persona nombra la carpeta de acá, como hoy. Un registro de NEW lee sus propios localizadores.
2. **El remoto.** Es el localizador que vale en cualquier máquina (D2, R16).
3. **El id.** Viene del mismo store, o es uno que ya viajó. Va por `canonical`, así que la lápida de una unión de acá lleva a su destino: es el test de la propuesta de un export hecho antes de una unión. Va antes que el camino porque el mismo camino puede tener otro repositorio mientras el id sigue nombrando el proyecto de antes.
4. **El nombre**, sólo cuando ningún lado tiene huellas. Para un proyecto sin carpeta ni remoto, el nombre es todo lo que tiene (PL32). Pero los nombres que da una carpeta (`api`, `web`, `docs`) se repiten entre proyectos distintos (D8). Si un lado tiene huellas y el otro no, el nombre no alcanza para unirlos: queda separado, con la sugerencia de D64. Un proyecto borrado no se elige por nombre, porque su nombre quedó libre (PL27).
5. **El camino**, el último, como hoy.
6. **Uno nuevo** (D63).

En los pasos 3 a 5, el proyecto del archivo no puede contradecir al de acá. La regla es la de `fit` (`project.rs:167-177`) sobre conjuntos, el remoto antes que el primer commit (D4). Una línea sin filas no crea proyectos: una lápida que no está acá no tiene nada que separar, y un repositorio sin filas sólo sirve para escribir su declaración.

**Lo que decide es puro.** `place(proyecto del archivo, registro, planes)` vive en un módulo nuevo, `crates/clawg-core/src/memory/portability/projects.rs`, con la lectura del registro. Se lee una vez por pasada del plan, en la misma llamada que `plan_ids_in`, como lo lee `stored_in` para `projects list` (`crates/clawg-core/src/memory/registry.rs:142`): tres tablas chicas, cuatro con `project_subfolder`. IL1 de `specs/portability` pide índices para lo que se busca por fila o por proyecto, y esto es una lectura por import. La transacción que escribe sigue `canonical` en cada fila, como hoy (`SqlSeat::live`, `import.rs:602-604`).

| Alternativa | Por qué no |
|---|---|
| El camino primero, como hoy (R4 de `specs/portability`) | En otra máquina el camino no existe, y en esta puede tener otro repositorio |
| El id primero | Un id sale del hash de un camino (`derive_project_id`): el mismo id en dos stores puede ser el mismo camino con otro repositorio. El remoto dice cuál es |
| El nombre aunque un lado tenga huellas | `api` de un repositorio y `api` de una carpeta sin git se unirían por un nombre que nadie eligió |
| Unir por el primer commit | Un fork y una copia lo comparten (D1, decisión 7 de la propuesta). Sólo vincula una mudanza (PL5) |
| Rechazar las filas de un proyecto que no se puede decidir | El import no puede preguntar. Separado y con una sugerencia, se arregla con una unión que tiene `--undo`; rechazado, obliga a editar el archivo |

## D63. Lo que se registra de un proyecto del archivo

- **Un proyecto nuevo conserva el id del archivo** cuando nadie lo tiene, ni en `project` ni en `project_path` (la comprobación de D40). Así, una vuelta por id sigue valiendo, y las filas que viajan sin línea (PL55) lo encuentran. Si el id está tomado, recibe `new_project_id` de su nombre. Su nombre es el del archivo. Hoy `record_projects` lo crea con el nombre que da el id (`import.rs:1251-1254`), y no lo anota en el diario.
- **Una carpeta de otra máquina** se graba con las huellas que trae el archivo, sin marca, y lleva al proyecto que decidió D62. Hoy se graba con lo que el plan leyó en este disco (`record_projects`, `import.rs:1242-1249`), que para un camino que no existe es nada. Con su primer commit, un checkout sin `origin` del mismo repositorio que se registre acá después es una mudanza (PL5) y llega a su proyecto. Y `projects list` muestra dónde vive el proyecto en otra máquina: el `root_path` informativo de la propuesta.
- **Una carpeta que existe acá** se graba como lo haría un registro (D12). Si su checkout contradice al proyecto del archivo, es otro repositorio y no se graba.
- **Cada remoto** del proyecto del archivo que nadie ata acá se ata al proyecto al que fue, así un clone que llegue después cae en él (R16). Hoy `record_projects` sólo ata el remoto que leyó en este disco (`import.rs:1257-1265`).

| Alternativa | Por qué no |
|---|---|
| No grabar las carpetas de otra máquina | Un checkout sin `origin` clonado acá después no tendría con qué llegar a su proyecto: PL5 necesita una carpeta con su primer commit cuyo checkout ya no está |
| Grabarlas con el proyecto de su plan, como hoy | En otra máquina el plan no lee huellas, así que la carpeta llevaría a un proyecto nuevo con su propio id, distinto del que decidieron el remoto, el id o el nombre |
| Un id nuevo para cada proyecto creado | La vuelta por id dejaría de valer, y las filas que viajan sin línea no lo encontrarían |

## D64. El import deja sugerencias, y las anota

La segunda pregunta de la propuesta. Cambia D10 y D12. D10 dejó afuera al import porque V26 no podía anotar sus sugerencias en su diario, y un `--undo` las habría dejado colgando. V31 las puede anotar (D69), y el `--undo` las retira mientras nadie las responda (D70).

**Cuáles deja** (PL58):

- **Las de un registro**, de cada carpeta que graba y existe acá: `root_commit`, `remote_changed` y `folder_name`. `plan_ids_in` (`project.rs:545-594`) pasa a devolver las que calcula `plan_in` (`project.rs:669-703`), con las mismas lecturas del disco: los checkouts grabados de los proyectos con ese nombre se miran por `ReadAhead`, como los de una mudanza.
- **`similar_name`.** El nombre de un proyecto del archivo se parece al de uno de acá, y no decidió el paso 4 de D62. Es la evidencia débil de la decisión 3 de la propuesta, la misma que lleva a un agente a preguntar (PL32).
- **`merged_at_source`.** La fuente tiene unidos dos proyectos que acá están separados: por una lápida del archivo, o por los remotos de un solo proyecto del archivo que acá llevan a dos.

Los motivos son dos valores nuevos de `project_suggestion.reason`, que no lleva `CHECK` (`V27__project_registry.sql:40-47`). Se dicen con palabras en los tres lugares que hoy dicen los demás: `suggestion_reason` (`crates/clawg-cli/src/mcp/tools.rs:755-765`), `reason` (`crates/clawg-cli/src/projects.rs:125-135`) y `candidate_reason` (`projects.rs:339-349`). Este último tampoco dice hoy `folder_name` con palabras, y lo suma.

| Alternativa | Por qué no |
|---|---|
| Que el import no deje sugerencias, como hoy | La copia o el remoto cambiado que registra un import se quedan sin sugerencia para siempre, y no se ven ni un nombre parecido ni una unión de la fuente |
| Unir lo que la fuente unió | Es una decisión que la persona tomó en otro store. Acá puede tener razones para separarlos, y unir mueve un proyecto entero. «Ante la duda, separado» (decisión 3 de la propuesta) |
| Sólo las sugerencias propias del import | El plan ya calcula las de un registro. Dejarlas afuera repite el hueco de hoy |

## D65. Importar otra vez después de un movimiento: la fila está en cualquier proyecto

La primera pregunta de la propuesta. Hoy una fila está presente si coinciden su agente, su proyecto, su hash y su fecha (`PRESENT_SQL`, `import.rs:1166-1169`). Después de `projects move`, el proyecto ya no coincide, y la fila entra otra vez en el proyecto de la línea.

Se elige buscarla en cualquier proyecto, por agente, hash, fecha y `created_by_ref`, y sólo para las filas de un export de clawg:

- **`created_by_ref`** es el agente que guardó la fila (`crates/clawg-core/src/memory/store_insert.rs:231`), y ni un movimiento ni una unión lo cambian. Una fila que vino de otra herramienta lleva su origen (`engram:<sync_id>`, HI7). Así se separa justo el caso que temía la propuesta: dos notas con el mismo texto y la misma fecha en dos proyectos de la fuente tienen dos orígenes.
- **Sólo clawg**, porque a una nota de otra herramienta ya la encuentra su origen, donde esté (HI11).
- **El índice** es `idx_memory_created_at`, sobre la fecha sola. Las filas que comparten un milisegundo son las piezas de un save, o notas de otra herramienta del mismo segundo, y lo demás se filtra encima. `idx_memory_agent_created` (`crates/clawg-core/migrations/V18__session_indexes.sql:13-15`) no sirve, porque es parcial y sólo tiene filas vivas: una fila movida y después borrada tiene que contar como presente, porque un import nunca trae de vuelta lo que el usuario borró (HI11).
- **El planificador** carga esas filas por fecha, en llamadas de `LOAD_BATCH` como las demás (ID1), y decide lo mismo que la pasada SQL (ID2).

**El riesgo** es una fila que la fuente tiene dos veces, con los cuatro campos iguales, en dos proyectos: acá queda una sola, en el primero. Hoy sólo el bug de esta pregunta produce esa forma. Importarla en otro store la arregla, y no pierde texto: la fila queda donde está.

| Alternativa | Por qué no |
|---|---|
| Aceptarlo, como hoy | Cada vuelta de un export a un store donde algo se movió duplica esas memorias. Entre dos máquinas que se sincronizan, cada movimiento |
| Buscar por el registro de movimientos de este store (`memory_move`) | No tiene falsos positivos, pero un movimiento hecho en el otro store duplica igual, y `memory_move` no tiene índice por proyecto de origen (`V21__memory_move.sql:21`) |
| Buscar en cualquier proyecto sin `created_by_ref` | Dos notas de otra herramienta con el mismo texto y la misma fecha, en dos proyectos de la fuente, quedarían en una |
| Un índice sobre `(content_hash, created_at)` | El hash ocupa 64 caracteres por fila, en cada save. La fecha sola ya deja pocas filas por clave |

## D66. Un proyecto borrado: una fila viva lo hace volver, y una borrada no

La tercera pregunta de la propuesta tiene tres partes:

- **`deleted_at` no viaja** (D61).
- **Una fila viva lo hace volver.** Si un proyecto del archivo llega por remoto, id o camino a un proyecto borrado de acá, sus filas vivas lo hacen volver, como cualquier escritor (D27): el mismo checkout lleva al mismo proyecto. El import lo anota (`revived`, D69) para que su `--undo` lo deje borrado otra vez, y su borrado sigue abierto con `back` mientras tanto (PL27).
- **`--project-map NOMBRE=project:ID` de un proyecto borrado responde 409 `project_deleted`**, como el destino de un movimiento (D35): la persona nombró un proyecto que para ella no existe.

Es la división que fijó la entrega 2b: nombrar un proyecto borrado se rechaza, usarlo lo hace volver.

**El arreglo.** Una fila que llega borrada no hace volver a su proyecto. El disparador no puede saberlo: cuando se inserta la fila, todavía no está marcada. Y marcarla antes de insertarla deja su texto en el índice, porque el disparador de inserción de V8 no mira `deleted_at`, y después el de borrado definitivo no lo resta (`crates/clawg-core/migrations/V8__memory_fts.sql:58-66`). Por eso, `SqlSeat::insert` lee el `deleted_at` y el `reviewed` del proyecto antes de insertar una fila borrada, y los deja como estaban después de marcarla. Es el único escritor que inserta viva una fila para borrarla (`import.rs:675-681`). D27 eligió disparadores para hacer volver un proyecto, porque olvidarse de un escritor deja una fila viva fuera de toda lista. Este caso es al revés, y sólo hay un escritor que lo necesita.

| Alternativa | Por qué no |
|---|---|
| Que una fila del import no haga volver un proyecto borrado | Las filas vivas del mismo checkout quedarían bajo un proyecto que no se lista. Es lo que D27 descartó |
| Un proyecto nuevo cuando el de acá está borrado | Dos proyectos para el mismo remoto o la misma carpeta |
| Arreglarlo en el disparador | Al insertar, la fila todavía no está marcada, y marcarla antes deja su texto en el índice (V8) |

## D67. Las declaraciones viajan en la línea de su repositorio

Una declaración es del proyecto del repositorio, no de un remoto (D51), así que viaja en su línea: (`subpath`, W). El import la escribe con los proyectos de acá, P' y W', cuando la clave (P', `subpath`) está libre, y la anota (D69). Una declaración de acá con la misma clave queda, aunque lleve a otro proyecto: la escribió la persona en esta máquina. Escribirla no une ni mueve nada (D56).

El subproyecto W se crea si hace falta, aunque no traiga filas: es la única línea sin filas que crea un proyecto (D62), porque la declaración sin su destino no dice nada.

Las declaraciones no pueden ir a `memory_import_extra`, porque su `CHECK` no tiene ese tipo (`crates/clawg-core/migrations/V26__import_journal.sql:60`).

| Alternativa | Por qué no |
|---|---|
| Una línea propia por declaración | Un lector de antes reporta cada una como inválida, y una declaración es un dato del repositorio (D51) |
| Reemplazar la declaración de acá | La escribió la persona en esta máquina, y reemplazarla cambia adónde va lo que se guarda desde esa carpeta |
| La clave por remoto en el archivo | Lo descartó D51: un repositorio sin `origin`, o con el remoto cambiado, no tendría subproyectos |

## D68. `--project-map NOMBRE=project:REF` y los caminos crudos

**La referencia.** Se resuelve como el destino de un movimiento (D47), con `find_project` (`registry.rs:871-888`): primero como id, después como el nombre de un solo proyecto vivo. No crea nada ni busca parecidos. Lo que no es ninguno da 404, lo que tienen dos o más da 409 `project_ambiguous`, y uno borrado da 409 `project_deleted` (D66). Todo eso se comprueba antes de leer el archivo. La lápida se sigue en la transacción que escribe (`SqlSeat::live`), como la sigue `target_in` en un movimiento. `MappedProject::root` (`crates/clawg-core/src/memory/portability/translate.rs:43-47`) pasa a ser opcional, y la traducción sólo escribe una línea `project` por una raíz (`translate.rs:641-646`).

**Los caminos crudos.** La 3b dejó el import como estaba: `--project-map NOMBRE=CAMINO` toma la raíz de CAMINO, y el import no manda el camino crudo (D59; PL47: «el import es de la entrega 4»). Se decide que lo mande, para `--project-map NOMBRE=CAMINO` y para `--map-root OLD=NEW`. `clawg projects move --to CAMINO` ya lleva a un subproyecto (PL47), y es raro que el mismo camino lleve a dos lugares según el comando. El CLI resuelve la raíz como hoy (`crates/clawg-cli/src/import.rs:60-62`) y manda las dos cosas.

**El cuerpo.** `ImportRequest` (`crates/clawg-cli/src/daemon/api.rs:1940-1959`) espera pares de dos textos, y no rechaza los campos que no conoce. Un campo nuevo pasaría en silencio por un daemon anterior, que guardaría las notas sin proyecto. Y un camino crudo en lugar de la raíz lo registraría como una raíz. Con un objeto como destino (`{"root", "path"}` o `{"project"}`), un daemon anterior rechaza el cuerpo sin escribir nada. Uno nuevo acepta además un texto, de un CLI anterior.

**Partir el argumento.** Por el último `=project:`, y si no lo hay, por el último `=`, como hoy (`crates/clawg-cli/src/cli.rs:339-346`). Un camino que contiene `=project:` se escribe con `./`.

| Alternativa | Por qué no |
|---|---|
| Crear el proyecto si el nombre no existe | Un nombre mal escrito crearía un proyecto vacío. `clawg projects create` busca parecidos antes (PL39) |
| Un campo nuevo en el cuerpo, al lado de `project_map` | Un daemon anterior lo ignora sin error y guarda las notas sin proyecto |
| Seguir mandando la raíz de CAMINO | Un CAMINO dentro de una subcarpeta declarada iría al repositorio, mientras `projects move --to` con el mismo CAMINO va al subproyecto |
| `project:` también en `--map-root` | Un proyecto del archivo ya va por su remoto, su id o su nombre (D62). El mapa es para la carpeta que cambió de lugar |

## D69. V31: una tabla de diario nueva y un índice por fecha

La pregunta de la propuesta sobre «los tipos nuevos de `memory_import_extra`». Se elige una tabla nueva, `memory_import_registry` (`import_id`, `kind`, `a`, `b`, `value`), sin `CHECK`, como `project_merge_change` (`crates/clawg-core/migrations/V28__project_merge.sql:32-37`). En `memory_import_extra`, el tipo `project` sigue queriendo decir «una carpeta que el import grabó» (D1), y `remote`, un remoto.

Sus columnas son texto, porque lo que anota son ids de proyecto y subcaminos. En `memory_import_extra`, `a` es un entero (`V26__import_journal.sql:61`), porque guarda el id de una fila.

V31 crea también `idx_memory_created_at` (D65). El test de la migración siembra un store en V30, con un import anotado, y comprueba que abre sin cambiar ninguna fila y que ese import se deshace como antes (PL53).

| Alternativa | Por qué no |
|---|---|
| Reconstruir `memory_import_extra` sin el `CHECK` | No es aditiva: la migración copia el diario de cada import. Y su `a` es un entero, que no guarda un id de proyecto |
| Una tabla por tipo, como `project_delete_row` (D28) | El `--undo` busca por import, no por fila. Serían cuatro tablas para unas pocas filas por import |
| Anotar en `project_merge_change` | Es el diario de las uniones, y mezclaría los dos `--undo` |

## D70. Lo que deshace `--undo`

Va en la última transacción del `--undo`, después de las filas, las carpetas y los remotos (IU2), en este orden: las declaraciones, las sugerencias, los proyectos que volvieron y los que creó. Un proyecto que creó el import sólo queda libre cuando ya se fueron sus filas, sus carpetas, sus remotos, sus declaraciones y sus sugerencias.

Cada cosa se retira sólo si sigue como la dejó el import (PL63):

- **Una declaración**, si todavía lleva al proyecto que escribió el import. La persona puede haberla declarado otra vez.
- **Una sugerencia**, si nadie la respondió. Una respuesta es de la persona.
- **Un proyecto que volvió** se deja otra vez borrado si sigue vivo, sin revisar y sin filas vivas, y si su borrado con ese `deleted_at` sigue abierto. Si después se deshizo ese borrado, el proyecto tiene que quedar vivo.
- **Un proyecto que creó** se borra si ya nada lo nombra y sigue sin revisar. Un `rename` lo marca revisado (D11).

Un remoto que el import ató a un proyecto que ya estaba sigue la regla de IU2: se retira sólo si no quedan filas bajo su proyecto, porque un registro posterior puede depender de él.

La decisión es pura, sobre lo que se leyó, como `decide` (`crates/clawg-core/src/memory/portability/undo.rs:284`). Lo que queda se nombra en el reporte, con su motivo.

| Alternativa | Por qué no |
|---|---|
| Borrar siempre lo que creó el import | Se llevaría un proyecto que recibió saves después, o la respuesta de la persona a una sugerencia |
| Dejar todo, como hoy (D12) | El `--undo` dejaría proyectos `import` vacíos y sugerencias colgando, que la sesión muestra (PL36) |

## Costos en el camino caliente de la entrega 4

- **Cada save** suma una entrada en `idx_memory_created_at`: la fecha y el rowid. Es el único costo fuera del export y del import.
- **El export** lee el registro una vez: cuatro tablas chicas.
- **El plan de un import** lee el registro una vez por pasada y decide en puro. Por cada carpeta que graba y existe acá, suma lo que ya lee un registro para sus sugerencias: un `stat` por checkout grabado de los proyectos con ese nombre o ese primer commit.
- **Cada fila de un export de clawg** que no está en su proyecto suma una búsqueda por `idx_memory_created_at`. En un store vacío son todas, por índice.
- **La migración** construye el índice una vez, con un recorrido de `memory`.

## Volver atrás de la entrega 4

- **El esquema.** Un binario anterior rechaza un store en V31 sin tocarlo (`SchemaNewer`), y la vuelta atrás es su snapshot `pre-migration`. Para quitar V31 a mano, con el daemon parado:

  ```sql
  DROP INDEX idx_memory_created_at;
  DROP TABLE memory_import_registry;
  DELETE FROM refinery_schema_history WHERE version = 31;
  ```

  Lo que un import agregó al registro queda, y el `--undo` de un binario anterior ya no lo retira: los proyectos que creó se borran con `clawg projects delete` (vacíos, al momento), y sus sugerencias se responden.
- **Un import.** Con `clawg import --undo N`, o con su snapshot `pre-import`.
- **El formato.** Un clawg anterior lee un archivo de esta versión: ignora los campos nuevos, reporta las líneas sin `root_path` y deja las filas de esos proyectos con su id, como antes.
