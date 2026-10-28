# ranking-at-scale — Diseño

El paso 1 entero (RS1 a RS12 de la [spec](spec.md)) y un esbozo condicional de los pasos 2 y 3 (RS13, RS14). Las citas son del código del 2026-10-28.

## Frente a la spec

La spec y el diseño se escribieron en paralelo. Las diferencias de esta tabla se resolvieron como dice la columna «Diseño», y la spec ya está alineada con ellas.

| Punto | Spec | Diseño | Por qué |
|---|---|---|---|
| Forma del caso | `expected_origins` (`created_by_ref`) y `project` por nombre (RS6, RS7) | Se adopta. El nombre se resuelve con `resolve_project` mismo (`registry.rs:848`), sin crear | Es la regla del argumento `project` de las herramientas MCP. Lee primero un id y después un nombre vivo; un nombre no choca con un id (`slug-4hex`, `derive_project_id`) |
| Origen | `engram:<sync_id>` | Se resuelve sólo entre filas con `source = 'import'` | Una save nativa guarda en `created_by_ref` el id del agente (`store_insert.rs:231`): miles de filas comparten `claude-code`. El índice parcial de V23 (`V23__origin_project.sql:18-19`) cubre esa condición |
| Salida de un fondo rehusado | 1 (RS2) | Se adopta, también para un esquema divergente, uno sin historia de clawg, uno dañado y uno sin contrato | 2 dice «arreglá el golden» y 4, 5 y 6 dicen otras cosas (`main.rs:49-59`). Un fondo inservible es una entrada que no se puede leer, como un fallo de E/S o de base, que es lo que dice 1. Un código nuevo no tendría quien lo lea |
| Errores de uso con `--background` (RS1) | con `--docs` y `--lang-detect` | Además con `--fts-tokenizer`, `--follow-up`, `--audit` y `--lexical-only`. Los `--sweep-*` sí se aceptan | `--fts-tokenizer` hace `DROP TABLE memory_fts` sin rellenarla (`eval_api.rs:353-377`): las filas del fondo saldrían del canal léxico. Los otros tres miden el corpus del golden, que el fondo no cambia, y `--lexical-only` busca con `Reach::Own` y sin proyecto (`search.rs:1410-1413`). El paso 2 necesita barrer sobre el fondo |
| Pool corto (RS9) | menos de 30 saves **alcanzables** en el alcance del caso | **Se discute:** menos de 30 saves en el pool que **devolvió** la KNN de `memory`, después de su reintento. Las alcanzables se informan como causa | Ver [D6](#d6-pool-corto-y-supervivencia) |
| `pool_survival` (RS8) | si la esperada llegó al pool y qué pasadas la sostuvieron | `{requested, saves, eligible, expected_in_pool}`, con `passes` aparte | Ver D6. «Qué pasada» no se informa |
| `best_cosine` (RS8) | coseno crudo del mejor hit no exacto | El **máximo** coseno crudo entre los hits no exactos devueltos | Con una fila igual a la consulta da 1 de las dos formas. El primero por score puede ser menos cercano pero más nuevo o importante, y C1 quiere justamente no mezclar decaimiento ni importancia |
| `decay_age_days` (RS8) | «la edad que lee el decaimiento» | Una lista alineada con `top5_types` | La decisión de B3 lee la edad de las saves que inundan (decisión 6 de la propuesta), no la de la esperada |
| Composición (RS3) | por tipo, agente, idioma y con o sin proyecto | Suma `by_source`, `by_project` (por nombre), `placement` y `schema` | El corte viejo trae filas nativas que la reimportación no tiene, y la decisión 1 pide la misma ubicación en los dos. Son nombres y cuentas, sin texto ni ids |
| Ids en resultados (RS8, RS11) | sin ids de fila | Un hit del fondo se escribe `bg`, sin su id | Hoy un hit sin ref se descarta (D5). Un marcador con el id rompería RS11 |
| Campos nuevos sin fondo (RS1) | nada nuevo sin `--background` | Los campos de RS8 y los agregados de RS10 salen sólo con fondo | RS1: «ningún bloque nuevo aparece» |

## D1. Abrir el fondo: copiar, sondear, decidir y recién después abrir

RS2 pide rehusar sin escribir en la copia. `MemoryStore::open` escribe antes de cualquier otra cosa: pone WAL (reescribe la cabecera), escribe un snapshot `pre-migration` y migra (`db.rs:203-242`). `ensure_embedding_contract` escribe la fila si falta (`db.rs:337`). Por eso todo lo que decide se lee antes de abrir, en sólo lectura.

`eval_api::copy_background(snapshot, dir, &EvalEmbedder) -> Result<EvalBackground, EvalError>`:

1. **`-wal`, sin abrir el original.** Se extrae de `check_restorable` (`backup.rs:508-517`) un `wal_with_commits(path) -> Option<PathBuf>`, que sólo lee metadatos. Lo llaman `restore` y esta función. El original nunca se abre como SQLite: RS1 lo prohíbe, y una conexión de sólo lectura sobre un archivo WAL crea `-shm` a su lado (`backup.rs:581-584`).
2. **Copia con hash.** Se lee el original como archivo, de a 1 MiB, se escribe `dir/memory.db` y se calcula el sha256 en la misma pasada (`sha2`, ya en uso por `sha256_hex`, `eval_api.rs:1011`). Una sola lectura de cientos de MB.
3. **Esquema e integridad, sobre la copia.** `check_restorable(copy)` pasa a `pub(crate)` y devuelve el `SchemaPlan` que leyó; hoy devuelve `()` (`backup.rs:501-541`) y `restore` lo ignora. Rehúsa la copia dañada (`quick_check`), sin historia de clawg, más nueva o divergente. Es la misma regla que `clawg restore`.
4. **Contrato, en sólo lectura.** `open_for_probe(copy)` (`backup.rs:585`; sin `-wal`, la abre `immutable`) e `inspect::contract` (`inspect.rs:436`, pasa a `pub(crate)`). Decide `background_contract(stored: Option<StoredContract>, actual: &EmbeddingIdentity) -> Result<(), BackgroundRefusal>`, que es pura. Sin fila da `NoContract`, que es también el caso de un store anterior a V5, donde nace la tabla. Una diferencia da `Contract { differing, pooling_upgrade }`, con `differing_fields` (`db.rs:88`) e `is_pooling_upgrade` (`bootstrap.rs:520`), para que el mensaje diga «un store CLS; abrilo una vez con el daemon y volvé a respaldarlo». No se reusa `verify_or_reembed`, que re-embebería un store CLS (`bootstrap.rs:544-561`), y RS2 lo prohíbe.
5. Devuelve `EvalBackground { sha256, bytes, schema_from, schema_to, contract }`.

Los rechazos son `EvalError::Background(BackgroundRefusal)`, con `thiserror` y variantes `WalWithCommits(PathBuf)`, `Unusable(String)`, `NoContract` y `Contract { … }`. `rag-eval` los convierte en salida 1.

**Un esquema más viejo se migra en la copia (pregunta 3).** Después de decidir, quien llama abre la copia con `EvalStore::open` o `EvalIngest::open`, que migran como el daemon. El `pre-migration` queda dentro de `dir` y se borra con él.

| Opción | A favor | En contra |
|---|---|---|
| **Migrar la copia (elegida)** | Mide el store como lo sirve este binario después de actualizar, que es lo que va a leer el usuario. El snapshot dev puede ser de un esquema anterior | El disco temporal llega a unas 2× del snapshot. Las migraciones que cambian datos (V27, el registro de proyectos) corren sobre la copia, y el informe lo dice con `schema_from` → `schema_to` |
| Rehusar todo lo que no esté al día | Mide sin migrar nada | Obliga a abrir `~/.clawg-dev` con un daemon nuevo antes de respaldarlo, o sea a escribir en el perfil |

**Leer el pool con migraciones pendientes (pregunta 4).** No pasa. Antes de abrir sólo se leen la historia y `embedding_contract`, y las dos existen en todo esquema que el paso 3 no rehúsa. La composición, la resolución, la búsqueda y la cuenta del pool corren después de `open`, con el esquema del binario. Lo que la migración no hace es el pase de mantenimiento: las filas anteriores a V22 tienen `memory.lang` NULL. La composición las cuenta como `null`, y el idioma de la save esperada se toma de `language::detect` sobre su texto, la misma función con que el pase llena la columna.

**Limpieza.** `rag-eval` es dueño de `dir`, un `rag-eval-<uuid>` del temporal (`seed.rs:102-104`). Con fondo, a diferencia del store sembrado de hoy, que queda en disco a propósito, se borra al terminar, también con error (RS1). Pesa cientos de MB, tiene texto personal y el original sigue ahí para repetir la corrida. Hace falta `EvalStore::close(self)`, que espera `conn.close()` como `release` (`runner.rs:223-236`), porque en Windows un archivo abierto no se borra. El borrado con reintentos de `remove_scratch` (`runner.rs:249-258`) pasa a un módulo compartido de `rag-eval`. La corrida queda en una función interna que devuelve el código, y la externa cierra y borra siempre, como `runner::run` (`runner.rs:261-285`).

## D2. Una sola búsqueda, con el alcance del daemon

`EvalStore::search(embedding, agent, scope, project: Option<&str>, k_final, scoring, query_text)` arma el alcance como `api.rs:1465-1469`. El alcance de agentes es `self.0.reader_for(agent).reach` (`db.rs:285-290`), que lee `sharing.json` del directorio del store como el daemon (`sharing.rs:85-94`). En la copia no hay ninguno, así que da `Reach::AllBut([])` (`sharing.rs:150-158`). El proyecto pasa por `canonical` (`merge.rs:54`), como `live_project` (`api.rs:1504-1509`). `EvalScope` sigue siendo `Copy` y `const` (`K_POOL`, `main.rs:39`): el proyecto es un argumento, no un campo.

| Opción | A favor | En contra |
|---|---|---|
| **Un camino, siempre con el alcance del daemon (elegida)** | Una sola búsqueda que mantener. Sin fondo, el store tiene un solo agente y ninguna fila con proyecto, así que `AllBut([])` y `Own` admiten las mismas filas y `project: None` no filtra. El ancla no se mueve (RS4: «salvo por el alcance de agentes») | El SQL cambia de texto (`hidden: []`, `search.rs:64-70`). Lo fija un test |
| Guardar `Reach::Own` para las corridas sin fondo | No cambia ni el SQL | Dos caminos, y el de producción no se ejercita en el ancla |

Test guardián: `a_one_agent_store_ranks_the_same_under_the_daemons_reach`, en `eval_api.rs`. Compara ids y scores de `EvalStore::search` contra `search_all_memories_with_scoring` con `Reach::Own` sobre el mismo store. `--follow-up` pasa `None`.

## D3. Lo que la búsqueda devuelve para el informe

- **`search.rs`.** `KnnOutcome` (`:427-444`) suma `saves`, la cuenta de `distinct_saves` sobre las filas recortadas con los vínculos que `knn_one` ya leyó (`:597`, `:614`), y `eligible: Option<usize>`, la cuenta que `knn_one` ya consulta cuando la ventana vuelve corta (`:598-606`). `search_all_memories_inner` (`:1211`) devuelve `SearchTrace { telemetry: String, memory: MemoryPool }`, con `MemoryPool { requested, rows, saves, passes, eligible, ids }`. La línea de log se arma igual (`format_pool_line`, `:664-679`). `search_all_memories` lo descarta (`:1401`), así que producción no cambia de orden ni de respuesta. Su costo es mover hasta unas decenas de `i64` por búsqueda. `search_all_memories_with_scoring` (`:1420`, sólo `eval`) lo devuelve.
- **`eval_api.rs`.** `EvalStore::search` devuelve `EvalSearch { hits, pool: EvalPool }`. `EvalHit` (`:203-213`) suma `memory_type` y `exact`. `EvalStore::last_accessed(ids) -> Vec<(i64, i64)>` lee `last_accessed_at` en una consulta aparte, como `row_distances` (`:698`). No se agrega a `RankedHit`, que se serializa en la respuesta del daemon (`search.rs:117-149`). `cosine_from_distance(d)` reexporta `cosine_from_l2_unit` (`score.rs:209`, pasa a `pub(crate)`), como el módulo ya reexporta las constantes del detector (`eval_api.rs:803-806`): la medición lee la función que se distribuye, no una copia.

## D4. Resolver los casos una vez, antes de medir (pregunta 5)

`EvalStore::resolve_cases(&[EvalCaseRef]) -> Result<Vec<EvalResolved>, EvalResolveError>`, y lo mismo en `EvalIngest`. Por dentro es una sola función sobre `&MemoryStore`. Corre una vez por fondo, después de abrir y antes de sembrar o buscar. Un golden inválido sale con 2 antes de medir nada (RS6, RS7), y cada búsqueda sólo consulta conjuntos.

| Opción | A favor | En contra |
|---|---|---|
| **Una vez, en la validación (elegida)** | Sale con 2 antes de gastar la corrida. Una consulta por origen sobre el índice parcial | Hay que llevar las filas resueltas hasta el puntaje (un `HashSet<i64>` por caso) |
| En cada búsqueda | No guarda nada | Repite consultas iguales por cada punto del barrido, y puede salir con 2 a mitad de corrida |

Por caso:

- **Proyecto.** Se resuelve con `resolve_project(store, name, ResolveOptions::default())`, sin crear. Una negativa (`ambiguous`, `similar`, `unknown`) o un proyecto borrado dan `UnknownProject` o `AmbiguousProject`, con los candidatos. El id que resulta es el de ese fondo (RS7).
- **Origen.** `SELECT … FROM memory WHERE source = 'import' AND created_by_ref = ?1 AND deleted_at IS NULL AND status IN ('active','disputed')` devuelve las filas vivas de la save: lo que la búsqueda puede devolver (`search.rs:266-267`). Sin ninguna, `NoLiveRow`, que incluye la save que una reimportación dejó `superseded`. Una fila con proyecto que no es el del caso tras `canonical`, que no es NULL ni `user_preference` (`memory_project_predicate!`, `search.rs:107-114`), da `OutsideProject` (RS7). Es lo que delata un snapshot que ubicó la save en otro proyecto.
- **Datos para el informe.** El `memory_type` de la save. Su idioma: `memory.lang` de la primera pieza o, si es NULL, `detect`. Su texto unido, con `join_pieces`, sólo para el matcher de agent-view (D8); nunca va al informe. Y para la excepción del paso 2, si la save es el `session_summary` que `LAST_SESSION_SQL` (`prompt_builder.rs:860`) mostraría en su proyecto (`expected_is_last_summary`).

Supuestos de la spec que este mecanismo necesita y cumple: `created_by_ref` es igual en los dos snapshots porque lo escribe la traducción desde el `sync_id` (`translate.rs:601`), y los ids y las piezas no. El nombre de un proyecto es el mismo en los dos mientras ambos ubiquen las filas igual (D10). Un caso `unanswerable` sólo resuelve su proyecto.

## D5. El corpus del golden encima, y cómo se puntúa

| Ruta | Cómo siembra con y sin fondo | Por qué |
|---|---|---|
| Ordenada (por defecto) | `EvalStore::insert`, bajo `rag-eval-agent` y sin proyecto, como hoy (`seed.rs:71-82`) | El `--diff` del ancla con y sin fondo tiene que diferir sólo en el fondo (RS12). Sembrar con `EvalIngest` en una de las dos cambiaría también el troceo, el dedup y el detector |
| Agent-view | `EvalIngest`, como hoy (`runner.rs:91-95`) | Igual razón, del otro lado |

**El rango honesto.** Hoy `ranked_refs` descarta todo id que no está en el `RefMap` (`filter_map`, `main.rs:445-446`). Sobre un fondo eso subiría la esperada por encima de cada hit del fondo e inflaría el recall sin que nada lo muestre. Con fondo, cada hit se escribe así:

1. el ref del golden;
2. si no, el origen que el caso cita, cuando la fila está en sus filas resueltas (D4);
3. si no, `bg`.

Un ref que ya apareció se escribe `bg` (dos saves vivas de un mismo origen), así que `recall_at_k` no pasa de 1 (`metrics.rs:31-37`). Un hit en cualquier pieza cuenta (RS6), y `collapse_saves` ya deja una pieza por save (`search.rs:167-184`). `abstained` (`main.rs:455`) queda en falso sobre un fondo, porque la búsqueda nunca vuelve vacía. Eso es lo que mide el paso 3.

**Gemelos.** Si el golden se escribió a partir de memorias reales, una de sus filas puede estar también en el fondo con el mismo `content_hash`. La ruta ordenada no le da crédito al gemelo (RS1: un hit del fondo nunca cuenta), y agent-view, que reconoce por texto, no puede distinguirlo. El informe lista esas refs en `background.golden_twins`, con una consulta por `content_hash` después de sembrar, para leer el ancla con eso a la vista.

- **Similitud.** El bloque de coseno puro sigue sobre los vectores del corpus (`main.rs:269-292`) y excluye los casos por origen, que no tienen vector de corpus.
- **Nombre del archivo.** El sha256 entra al tag como `bg-<8 hex>`, plegado como el del tokenizer (`tokenizer_slug`, `fold_tag`, `main.rs:1083-1104`). Así el ancla con y sin fondo del mismo día no chocan en la salida 4 (RS11).

## D6. Pool corto y supervivencia

**La definición (preguntas 1 y 2, y lo que se discute de RS9).** Corto es `memory.saves < memory.requested`: el pool de la KNN de `memory` vuelve, después de su único reintento (`search.rs:612-615`), con menos saves distintas que las 30 que pide (`SearchScope::all_default`, `:36-46`).

| Opción | A favor | En contra |
|---|---|---|
| **El pool devuelto (elegida)** | Captura el crowding, la razón de excluir casos que da la propuesta (riesgo «pocos casos útiles», knn-prefilter). No cuesta ninguna consulta: sale del `KnnOutcome`. vec0 corta la ventana antes de filtrar estado, agente y proyecto (`search.rs:244-251`), y con un proyecto chico entre ~52.000 vectores la ventana de 30, o de 120 en el reintento, puede volver con menos de 30 saves del alcance | Mezcla dos causas. `eligible` las separa |
| Saves alcanzables en el alcance (la de RS9) | Es una propiedad del alcance, no de la ventana | Todo proyecto lee las filas sin proyecto y toda `user_preference` (la importación de prod dejó ~15.709 sin proyecto, precondición de la propuesta). Así, ningún alcance de un fondo real baja de 30: nunca se dispararía y el crowding pasaría sin ver. Contarlas por caso, con `DISTINCT` sobre la save y `LIMIT 30`, es barato, pero mide otra cosa |

**La causa.** `eligible` es la cuenta de filas que `knn_one` ya pide, con `LIMIT k`, cuando la ventana vuelve corta (`:598-606`). Si es menor que 30, el alcance no tiene más: `short_pool_cause: "scope"`. Si no, la ventana estaba tapada o era de las piezas de pocas saves: `"crowded"`. Las dos quedan fuera de los agregados de RS10 y se cuentan en `short_pool_cases`, con la causa.

**`pool_survival`** es `{requested, saves, eligible, expected_in_pool}`, y `passes` (1 o 2) va aparte, como pide RS8. `expected_in_pool` dice si una fila de la save esperada estaba entre los `ids` del pool de `memory` (D3). Es `None` para un `unanswerable`, y falso si sólo llegó por el canal léxico. No se informa qué pasada la trajo. Saberlo pediría guardar los ids de la primera ventana en cada búsqueda de producción, y no cambia ninguna decisión. Lo que importa es si la falla es de pool (no llegó) o de orden (llegó y quedó abajo). Coincide con lo que el código ya llama supervivencia del pool: pedido contra devuelto, y si hubo reintento (`format_pool_survival`, `admin.rs:98-116`).

## D7. Informe y agregados

Todo con `#[serde(default, skip_serializing_if = …)]`, así que un archivo viejo sigue leyéndose en `--diff`, y sin fondo no aparece nada nuevo (RS1).

- **`RunResults.background`.** `path`, `sha256`, `contract`, `schema {from, to}`, `searched_at_ms` y `composition`, más `golden_twins`. La composición cuenta las filas vivas por `memory_type`, `agent_id`, `lang` (con `null`), `source`, con o sin proyecto y `by_project` por nombre, y su `placement` (`origin_project` → nombre de proyecto o `null`, filas). Es un `GROUP BY` por eje en `eval_api`, antes de sembrar.
- **Por caso** (`CaseResult`, `report.rs:62-97`):
  - `project`: el nombre que dio el caso;
  - `expected_type`;
  - `top5_types`, y `decay_age_days` alineado con ellos, contado desde el `now` que la corrida lee justo después de cada búsqueda;
  - `best_cosine`;
  - `score_gap_1_10`, ausente con menos de 10 hits;
  - `passes`, `pool_survival`, `short_pool` y `short_pool_cause`.

  `lang` se llena para los casos por origen con el idioma de D4, así que `by_pair` los corta sin cambios. Los `unanswerable` llevan los mismos campos en `retrieval.unanswerable`, que ya existe aparte (`report.rs:242-245`).
- **`RetrievalResults.scale`**, con funciones puras sobre `per_case` (RS10):
  - `top3_by_type`: los lugares del top 3 por tipo en los casos activos, no cortos, cuyo `expected_type` no es `session_summary`, junto a la parte de filas vivas del tipo en la composición;
  - las distribuciones (n, p5, p25, p50, p75, p95) de `best_cosine` y `score_gap_1_10`, por con o sin respuesta × pareja × partición (`split_of`, `report.rs:180`);
  - `short_pool_cases`.

## D8. Agent-view

- **Fondo.** `copy_background(snapshot, scratch, …)` antes de `EvalIngest::open(scratch)` (`runner.rs:91`). El scratch ya vive en el temporal, y `run` ya lo borra ante cualquier error que no sea `OutsideTemp` (`runner.rs:261-285`). Después: `resolve_cases`, la composición y recién ahí el corpus.
- **Contrato al abrir.** `EvalIngest::open` llama a `ensure_embedding_contract` y falla ante `Mismatch`. Hoy pone el embedder sin comprobarlo (`eval_api.rs:566`), así que su store queda sin la fila de contrato que el daemon escribe al cargar el modelo. Con ella, un store hecho por `EvalIngest` sirve de fondo en los tests. Sobre una copia ya decidida da `Match`.
- **Edad (RS5).** `send(…, age_ms)` adelanta el `timestamp_ms` del evento (`explicit_event`, `eval_api.rs:515-518`). El worker guarda una save con el instante de su evento (el test de `worker.rs:2888-2924` siembra una de 30 días). El ancla no se mueve por esto: ni `docs/eval/golden.json` ni `golden-expanded.json` tienen `age_days`.
- **Proyecto (RS4).** El id resuelto va en `SearchArgs.project_id` y en `context` (`runner.rs:140`, `:153`). El alcance de agentes ya es el de producción, porque el daemon lee `sharing.json` del scratch y no hay ninguno (`api.rs:1465-1469`).
- **Origen.** El `Corpus` del matcher suma `(origen, texto unido)` por cada origen citado. Un fragmento visible de 40 caracteres o más dentro del texto unido acierta, en cualquier pieza (`matcher.rs:47-49`). Las filas del fondo no entran al `Corpus`, así que no cuentan salvo un gemelo (D5).
- **Lo que también mide.** El corpus entra al store como una save: el dedup y el detector lo comparan con las filas sin proyecto del fondo que el agente alcanza. Las fusiones y los pares que eso deja son parte de lo que ve el agente.

## D9. El reloj

La búsqueda lee la hora al correr (`search.rs:1248`) y fijarla está fuera de alcance (propuesta, No-alcance), así que nada puede forzar la regla de RS12. Se registra y se avisa. El informe guarda `date`, que ya existe, y `background.searched_at_ms`. `--diff` imprime el sha256 y la hora de cada lado. Cuando los dos leyeron el mismo fondo con más de 24 h entre medio, imprime un aviso que nombra RS12 y las horas, sin cambiar el código de salida: `--fail-on-regression` sigue siendo sólo sobre las métricas.

## D10. Procedimiento: los dos snapshots (verify, no código)

Necesita Windows o Unix, un build debug de `clawg` con la feature `testing` de clawg-core en `target-test` (sólo ese build lee `CLAWG_TEST_HOME`, `paths.rs:168-191`), el modelo real, y autorización para leer `~/.clawg-dev` en el momento. Sigue a embed-throughput (`verify.md:11-20`).

1. **Corte viejo.** Con autorización: `clawg backup --profile dev`, y copiar el archivo a un directorio de trabajo fuera del repo, en el temporal, como `old-cut.db`.
2. **Su ubicación.** `rag-eval --real --background old-cut.db --golden docs/eval/golden.json --tag old-cut`. La composición da `placement` (qué `origin_project` quedó en qué proyecto, por nombre) y `by_source`. Si un `origin_project` está repartido entre proyectos, los casos que citen sus saves fallan con `OutsideProject`.
3. **Home aislado.** `CLAWG_TEST_HOME=<tmp>/ras-home`. Con autorización, copiar `~/.clawg-dev/models` a `<tmp>/ras-home/.clawg-dev/models`. Arrancar `clawg serve --profile dev --port 0`: la ruta del socket supera `sun_path` y sirve por TCP, y el cliente lee el puerto de `daemon.json`.
4. **Los mismos proyectos.** Por cada nombre de `placement`, `clawg projects create "<nombre>" --profile dev`.
5. **Reimportar con el corte nuevo.** `engram export --all <tmp>/engram.json`, fuera del repo, y después `clawg import --from engram <tmp>/engram.json --project-map "<origin>=project:<nombre>" … --profile dev`, una entrada por cada fila de `placement` con proyecto. Las que no tienen proyecto no se nombran. Con `project:` la partición queda igual sin depender de carpetas ni remotos; los ids pueden diferir, y por eso el caso nombra el proyecto.
6. **Snapshot.** `clawg backup --profile dev`, copiarlo como `new-cut.db`, `clawg stop --profile dev`, y borrar el home y el export.
7. **Comprobar.** Una corrida sobre `new-cut.db` tiene que dar el mismo `placement`, en filas por proyecto, salvo las notas que engram sumó o borró desde la importación vieja.

El export con que se importó el store dev se borró al terminar el verify de 85 (`verify.md:20`). El nuevo puede traer notas de más o de menos: una save citada que falta en un corte da `NoLiveRow` (salida 2) y el caso se saca del set. Si `by_source` muestra en el corte viejo una parte de filas nativas que no es chica, se dice al leer la atribución. Se pueden traer con `clawg export --profile dev --agent <agente>` e importar al home aislado, pero su ubicación pasa por las reglas de `import` y hay que comprobarla igual.

## Orden de commits

Cada uno compila, pasa la suite y empieza por sus tests en rojo.

| # | Commit | Rojo con que empieza |
|---|---|---|
| 1 | `refactor(core): hand back a search's memory pool as data` | `the_memory_pool_counts_saves_and_reports_its_eligible_rows` (`search.rs`). Los tests de la línea de log siguen pasando |
| 2 | `feat(core): search an evaluation store with the daemon's reach and a project` (`EvalStore::search`, `EvalSearch`, `EvalHit`, `last_accessed`, `cosine_from_distance`; `main.rs` pasa `None`) | `production_reach_reads_other_agents_and_unowned_rows`, `a_project_search_reads_its_own_unowned_and_every_preference`, `a_one_agent_store_ranks_the_same_under_the_daemons_reach` |
| 3 | `feat(core): backdate the saves of the evaluation ingest` | `eval_ingest_backdates_created_at` (RS5) |
| 4 | `feat(core): copy a backup snapshot into an evaluation store or refuse it untouched` (`copy_background`, `wal_with_commits`, el plan de `check_restorable`, `EvalStore::close`, composición, contrato en `EvalIngest::open`) | `a_snapshot_with_a_nonempty_wal_is_refused_untouched`, `a_snapshot_of_another_embedding_contract_is_refused_untouched`, `a_newer_schema_snapshot_is_refused`, `an_older_schema_snapshot_is_migrated_in_the_copy` (con `seed_on_disk`, `db.rs:482`), `the_composition_counts_rows_without_text` |
| 5 | `feat(core): resolve a case's project by name and its expected save by origin` | `a_project_name_resolves_to_each_backgrounds_own_id`, `an_ambiguous_or_unknown_project_name_is_refused`, `an_origin_resolves_to_every_live_piece_of_its_save`, `an_origin_without_a_live_row_is_refused`, `an_expected_save_outside_the_case_project_is_refused`. Fondo armado con `import_foreign`, el embedder falso y el chunker por bytes |
| 6 | `feat(eval): measure over a copy of a snapshot with --background` (`cli.rs`, `main.rs`, `report.rs`, ayuda) | RS1: `background_parses`, `background_with_docs_is_a_usage_error` (y los otros modos), `a_background_run_leaves_the_snapshot_byte_identical`, `the_background_copy_is_removed_even_when_the_run_fails`, `a_run_without_background_measures_as_before`; RS2 con salida 1; RS3 `the_report_describes_the_background_without_text`; RS11; y `a_background_hit_ranked_first_keeps_its_place`, que hoy desaparece por el `filter_map` |
| 7 | `feat(eval): cite a case's save by origin and name its project` (`golden.rs`, `main.rs`) | Los de RS6 y RS7. El fondo con orígenes se arma en un `#[cfg(test)]` de `rag-eval`: un export `clawg-memories` v1 escrito en el test e importado por el router en proceso (`serve_ephemeral`, `DaemonClient::import`), que conserva `source`, `created_by_ref`, `origin_project` y piezas, sobre un `EvalIngest` falso, cerrado después |
| 8 | `feat(eval): report each case's types, cosine, gap, pool and ages` | RS8, RS9 y RS10 |
| 9 | `feat(eval): run agent-view over a background with each case's project` | `agent_view_sends_the_case_project_to_search_and_context` y un caso por origen hallado en una pieza |
| 10 | `feat(eval): name each run's background in --diff` | `diff_compares_an_anchor_run_with_and_without_background` y `a_diff_over_one_background_a_day_apart_warns` |
| 11 | `docs: rag-eval --background and cases by origin` | Ninguno: es la sección Testing de `.claude/CLAUDE.md` |

## Pasos 2 y 3 (condicionales)

**Paso 2, si abre la puerta 1.** Lo decide `scale.top3_by_type` sobre el corte nuevo, en calibración: si `session_summary` ocupa más del 25 % de los lugares.

- **B1, el peso.** `ScoringConfig` suma `type_weights: TypeWeights { session_summary }`, con 1,0 en `DEFAULT`, así que es bit a bit igual a hoy y lo fija `eval_scoring_config_default_matches_…` (`eval_api.rs:1198`). `score()` lo multiplica al final (`score.rs:307`) en una fila de `memory` de ese tipo. Como `knn_one` ordena y recorta por score (`search.rs:641-642`), el peso también cambia qué saves sobreviven al pool, y es a propósito. El espejo es `EvalScoringConfig.session_summary_weight`, con `with_session_summary_weight`; la bandera `--sweep-summary-weight <w>`, repetible; un eje más de `build_grid` y el slug `sumw0.50`. `the_contradiction_pool_ignores_the_type_weight` (RS13) fija que el pool de 5 del detector no cambia.
- **B2, si B1 no llega.** Un tope por tipo en la lista mostrada, con una función pura aplicada después del corte del daemon (`api.rs:1486-1489`) y en el contexto (`prompt_builder.rs:232-241`).
- **B3, de control.** No es barrible hoy. Pide dos campos de espejo (la tasa de `session_summary` y el piso de tier 2) y dos banderas. El piso de tier 2 (`score.rs:279-283`) es de toda fila de tier 2, no sólo de los resúmenes, así que ese brazo no es por tipo. Entra si la mayoría de los `decay_age_days` de los resúmenes del top 3 no llega a 209 días.
- **La excepción.** Por defecto (iii). La (i), un argumento de tipos en `SearchRequest` y `SearchMemoryArgs` con los dos contratos bendecidos otra vez, sólo si hay casos minados con `expected_type` `session_summary` y `expected_is_last_summary` falso.

**Paso 3, si abre la puerta 2.** Lo deciden las distribuciones de RS10 con y sin respuesta, por pareja, en calibración, y U3 en prueba (cota de Wilson publicada, 0,584 para 16 de 20).

- **C1.** Una función pura en el núcleo, `abstains(best_cosine, umbral)`, con un umbral `ABSTAIN_COSINE` fijado por un test que nombra su contrato, como el del piso (`score.rs:411-416`). El daemon la aplica junto a `shown_hits` (`api.rs:1486-1489`): si se dispara, se retiene todo hit no exacto, sin `MIN_SHOWN_HITS`, y `withheld` los cuenta. El contexto la aplica antes de `retain(score > 0)` (`prompt_builder.rs:236-238`). El puente no cambia: una respuesta sin hits y con `withheld` ya imprime «no relevant memories for this query» (`tools.rs:109-114`), y un daemon anterior se lee como hoy. El `Hit` del cliente no trae `distance` (`client.rs:194-224`), por eso la puerta no vive en el puente.
- **C2.** Necesita el score del 10º. Con `limit` 5, el daemon buscaría al menos 10 y recortaría después.

## Riesgos

- **El corte viejo puede no tener orígenes.** Si el store dev se importó antes de que las filas llevaran `source = 'import'` y `engram:` (`by_source` lo dice), los casos por origen no corren ahí, y la atribución al corte se hace sólo con el ancla.
- **Gemelos.** Si el golden sale de memorias reales, el ancla con fondo pierde los casos cuya fila tiene gemelo (D5). Se ven en `golden_twins`.
- **Disco temporal.** La copia más su `pre-migration` llegan a unas 2× el snapshot por corrida, y se borran al terminar.
- **El crowding deja pocos casos.** Si muchos casos salen `crowded`, quedan pocos para concluir (riesgo de la propuesta). `short_pool_cause` dice cuántos son por eso.
