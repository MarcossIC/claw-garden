# history-import — Diseño

Los enfoques de fondo están en la [exploración](exploration.md#enfoques-posibles): se lee el export JSON de la fuente (A) y se traduce y se parte en el daemon (A1). Aquí van las decisiones de implementación.

## D1. Tres piezas puras y el camino de 43

En `crates/clawg-core/src/memory/portability/`:

1. `foreign.rs`, el modelo común: `ForeignExport` (la fuente, su versión, las notas, las relaciones, los conteos por campo de lo que no tiene lugar y cuántas notas reescribió la fuente), `ForeignNote` (origen, clase `Observation | Prompt | SessionSummary`, tipo de la fuente, título, contenido, proyecto, si es `personal`, clave, sesión, fecha del texto, accesos, última vista, borrado, fijada) y `ForeignRelation` (origen de las dos notas, verbo, estado del juicio, si la marcó el usuario, motivo, evidencia, fechas, si fue reemplazada). Sin I/O ni dependencias del store.
2. `engram.rs`, el adaptador: `read(text) -> Result<ForeignExport, ForeignError>`. Lee con `serde_json` structs propios del formato, con `#[serde(default)]` en lo opcional. Las listas de nivel superior son `Option<Vec<_>>`: la fuente escribe `null` para una lista vacía, y `#[serde(default)]` cubre una clave ausente pero no un `null`, que serde rechaza (`invalid type: null, expected a sequence`). Cada struct junta en `#[serde(flatten)] extra: BTreeMap<String, Value>` las claves que no lee, y el adaptador cuenta por nombre cada una que no es `null` ni vacía, sumada a los campos conocidos sin lugar (`tool_name`, `review_after`, `superseded_by_sync_id`): un campo que la fuente agregue después aparece en el reporte en vez de perderse sin aviso.
3. `translate.rs`, la traducción común: `translate(&ForeignExport, &TranslateOptions, &ChatChunker, now_ms) -> Translated`. Devuelve un `Parsed` de líneas `clawg-memories` (filas `memory`, `piece` y `conflict`, con ids sintéticos), el plan de cada nota (su origen, su fecha, si se acotó, el rango de sus filas) y la parte del reporte que no depende del store. Es pura salvo el chunker, que corta sin I/O con el tokenizer que ya tiene.

Después, `import.rs` sigue por las tres pasadas de 43 (`prepare`, `place`, `write_links`) con dos agregados: el reconocimiento por origen (D3) y las transacciones por nota (D4).

| Alternativa | Por qué no |
|---|---|
| Un camino de escritura propio para notas | Repetiría `place` y `write_links` (piezas, pares, cadenas, gemelos, claves tomadas), que ya resuelven idempotencia y vínculos entre transacciones |
| Traducir a texto JSON Lines y volver a leerlo | Un ida y vuelta sin uso: `Parsed` es lo que `prepare` recibe |
| El adaptador en `clawg-cli` | Partir y enmascarar como un save necesita el chunker del daemon (A1 de la exploración) |
| Ignorar los campos que el adaptador no lee | Un campo nuevo de la fuente se perdería sin que el reporte lo diga, contra «todo entra, sin pérdidas» |
| Una lista cerrada de campos sin lugar | Cuenta sólo los que se conocían al escribirla: el próximo campo de la fuente se pierde igual |

## D2. Ids sintéticos en orden de fecha

La traducción ordena las notas por fecha del texto y, a igual fecha, por origen, y numera sus filas desde 1 en ese orden: la primera fila de cada nota antes que las demás (`rows_are_numbered_by_the_date_of_their_note`). El número de línea de cada fila es su id, así el import lleva lo que dice por línea a la nota de esa fila. `place` inserta por id del archivo, así que los ids de acá siguen las fechas, y el par de una relación tiene `new_id` en la nota más nueva, como lo escribe el detector. `write_links` escribe cada par como `(mayor, menor)` (`RECORD_PAIR_SQL`, `crates/clawg-core/src/memory/dispute.rs:300`), también para un archivo de clawg, donde ya vienen así salvo que un extremo estuviera presente con un id viejo.

Las filas de una nota llegan enmascaradas por la nota entera, así que `prepare` no las vuelve a enmascarar (`Row.redacted` sale de la nota). Volver a pasar `redact` por una pieza podría enmascarar lo que el texto entero no (una marca después de una comilla invertida que el corte dejó al principio), y la pieza dejaría de ser la de un save.

## D3. El reconocimiento por origen

El origen va en `created_by_ref`, que el export de 43 ya lleva, y V23 lo indexa sólo donde vale (`WHERE source = 'import'`). La versión de una nota es su fecha del texto: engram la actualiza en cada reescritura (`updated_at`), y un import la guarda como `created_at` de cada fila.

`place` decide una vez por nota, al llegar a su primera fila, con las filas de su origen (`SELECT … WHERE source = 'import' AND created_by_ref = ?`), y guarda la decisión en el `Ledger` para las demás filas de la nota:

| Lo que hay de su origen | Decisión |
|---|---|
| Nada | Nueva: las reglas de 43 en su destino |
| Una fila con su fecha, o la fecha se acotó y hay alguna | Presente: cada fila va a la fila de su origen con su hash, o, si se fundió con otra nota, a la que le da `TWIN_SQL` |
| Sólo filas borradas | Presente (el usuario la borró) |
| Filas no borradas con otras fechas, y la nota borrada en la fuente | Presente, reportada |
| Filas no borradas con otras fechas | Revisión de la más nueva: en su proyecto y con su clave; más nueva, viva y reemplaza a las vivas del origen que no comparte; más vieja, reemplazada por la primera viva |

Para una revisión, la búsqueda del titular de la clave deja fuera las filas de su propio origen, que la revisión reemplaza en vez de ser reemplazada por ellas.

| Alternativa | Por qué no |
|---|---|
| El origen solo | Una nota que la fuente reescribió después no entraría nunca |
| El hash de la primera pieza | Una reescritura que cambia el final de una nota larga no se vería |
| Los hashes de todas las piezas | Depende del corte y pide comparar conjuntos; la fecha ya dice lo mismo para una fuente que la actualiza |
| Una tabla de orígenes | Una tabla y una migración más para lo que `created_by_ref` ya lleva y el export ya transporta |

## D4. Transacciones por nota

El pase que escribe junta notas enteras hasta llegar a `WRITE_BATCH` filas (`note_batches`, pura), y cada lote es una transacción: una nota y lo que su revisión reemplaza nunca quedan partidos entre dos. Un corte entre lotes deja notas enteras, que el import siguiente reconoce como presentes. Los vínculos siguen en la última transacción, como en 43; si falta, el import siguiente los completa, porque una nota presente lleva sus filas a las de su origen.

Los vínculos de piezas de una nota se escriben salvo que una fila de su origen ya tenga uno. La regla de 43 («ninguna de sus filas tiene una pieza») dejaría sin piezas a una nota que comparte una pieza con otra, cuando el worker deja que una fila sea pieza de dos saves.

## D5. Claves

`rewrite_key` (pura, en `translate.rs`): minúsculas, cada carácter fuera de `[a-z0-9._-]` y `/` pasa a `-`, se quitan los segmentos vacíos. Si eso la cambió, o pasa de `MAX_TOPIC_KEY_LEN` (`crates/clawg-core/src/memory/topic.rs:21`), se corta para dejar lugar y se agrega `-` y 10 hex del SHA-256 de la original (`sha2`, ya dependencia de `clawg-core`). Una original que no deja nada da `key-<hash>`. Diez hex son 40 bits: dos originales distintas no se vuelven una en ningún store de este tamaño.

La calificación se decide sobre el archivo entero: se agrupan las notas por destino y clave final; un grupo con más de un grupo de origen califica cada nota con `<grupo>/<clave>` y la vuelve a pasar por `rewrite_key`. Se califican todas y no sólo la segunda, así el resultado no depende del orden del archivo.

El segmento de un grupo es el nombre tal cual cuando ya es un segmento válido, salvo que sea `no-project` o termine en `.personal`: esos dos se leerían como el grupo sin proyecto o como el `personal` de otro nombre, y un nombre que hubo que reescribir podría coincidir con otro. En los tres casos lleva `-` y el hash del nombre, con la misma regla que una clave. Una clave calificada todavía puede caer sobre la de una nota de otro grupo (`a` con la clave `b/k`, `b` con `k`), así que la calificación se repite hasta que ningún destino tenga una clave de dos grupos; cada pasada alarga las claves que toca, y un tope de ocho sólo asegura que el ciclo termine.

Dos notas vivas del mismo grupo de origen y destino con una clave: la de fecha más nueva queda viva, y cada otra lleva en su línea `memory` `status` `superseded`, `superseded_by` la primera fila de la nueva y causa `revision`, que `write_links` ya traduce.

## D6. Notas de igual texto

Se agrupan las notas vivas por grupo de origen y hash del texto enmascarado. La de fecha más vieja (y origen menor) queda con la suma de los `duplicate_count`, el `last_seen_at` más nuevo y la marca `pinned` de cualquiera; las demás no dan filas, el reporte las nombra y sus relaciones van a ella. Un segundo import arma los mismos grupos, encuentra presente a la que quedó y no inserta las otras, con cualquier mapa. Por grupo de origen y no por destino: el destino cambia con el mapa, el grupo de origen no.

## D7. Fechas

`parse_utc` (pura, en `foreign.rs`) lee `YYYY-MM-DD HH:MM:SS` con una fracción opcional de hasta 9 dígitos, las dos formas que engram escribe (sin fracción al guardar una nota; con nanosegundos al borrarla o revisarla, y en `exported_at`), y RFC 3339 (`T`, fracción opcional, `Z` o un desfasaje). La fracción se trunca a milisegundos, sin redondear, así una fecha nunca pasa al segundo siguiente y la misma da siempre el mismo `created_at`, que es lo que compara el reconocimiento por origen (D3). Devuelve milisegundos UTC con `days_from_civil` de `crates/clawg-core/src/memory/backup.rs:252`, que pasa a `pub(crate)`. No se agrega una biblioteca de fechas para dos formatos.

## D8. Relaciones

La tabla de HI14 es una función pura (`pair_for`). Los extremos son las primeras filas de las notas, después de los grupos de igual texto. Para `supersedes`, la nota destino lleva en sus filas vivas `superseded`, `superseded_by` la primera fila de la fuente y causa `verdict`, y el par va juzgado; para `conflicts_with` (y `supersedes` protegido), la primera fila de la nota más nueva lleva `disputed` y la otra en `disputed_against`, y el par va `escalated`. `write_links` ya activa una `disputed` sin par. Entre dos notas, vale la relación vigente más nueva; las reemplazadas se cuentan.

| Alternativa | Por qué no |
|---|---|
| Contarlas sin guardarlas | Pierde juicios que clawg sabe guardar |
| Aplicar `supersedes` siempre | Saltearía la regla que pide al usuario para reemplazar una preferencia o una decisión (63) |

## D9. El tokenizer

El daemon arma el chunker con `ChatChunker::from_cell(runtime.tokenizer)` (`crates/clawg-core/src/memory/chunker/chat.rs:64`), la misma celda del worker. Si está vacía, `POST /v1/import` con otra fuente responde `503` `tokenizer_not_ready`, también en un ensayo: el corte por bytes daría otras piezas y el reporte no sería el del import.

## D10. La ruta, el comando y el reporte

`ImportRequest` suma `from` (un enum, `clawg` por defecto) y `project_map`. `ImportOptions` suma el mapa; `import_foreign(store, embedder, chunker, backups, export, options)` recibe el export que el daemon ya leyó con el adaptador de su fuente (lo lee antes de pedir el embedder, HI16, y así no lo lee dos veces, y `clawg-core` no elige adaptador), lo traduce en `spawn_blocking`, porque enmascarar y partir miles de notas no debe frenar las demás rutas, y sigue por el mismo camino que `import`. `ImportReport` suma `by_lang` y `foreign: Option<ForeignReport>`; un cliente anterior ignora los campos. Con otra fuente, `merged`, `topic_taken` e `invalid` quedan vacíos y lo mismo va en `foreign` por nota, nombrada por su origen (`with_foreign`), y las notas de un proyecto cuya raíz es de otro, por ese proyecto.

En el CLI, `--from` es un `ValueEnum`; `--project-map NOMBRE=RUTA` se corta en el último `=` (un nombre de proyecto puede tener `=`, una ruta de Windows no) y `RUTA` se resuelve con `resolve_project_root`. clap rechaza `--map-root` con `--from engram` y `--project-map` sin él.

## D11. V23 y los filtros

```sql
ALTER TABLE memory ADD COLUMN origin_project TEXT;
CREATE INDEX IF NOT EXISTS idx_memory_origin_project ON memory (origin_project)
    WHERE origin_project IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_memory_import_origin ON memory (created_by_ref)
    WHERE source = 'import';
```

`MoveFilter` y `ListParams` suman `origin_project`, una igualdad más en el SQL que ya arman (`relocate.rs:490`, `push_filters` en `admin.rs`).

## D12. El alcance del export

Las versiones nuevas de engram exportan sólo el proyecto del directorio en que se corren, salvo con `--all`, y el archivo no dice qué alcance tuvo. El import no lo adivina: un export vacío se importa sin error y sin filas, y uno parcial se ve en las notas por proyecto de la fuente del reporte (HI17), que `--dry-run` muestra antes de escribir. El comando y las docs piden `engram export --all`.

| Alternativa | Por qué no |
|---|---|
| Rechazar un export vacío | No protege ningún dato: no escribe nada igual, y el reporte ya dice que no trae notas |
| Avisar cuando todas las notas vienen de un proyecto | Exportar un solo proyecto es válido, y el reporte ya lo muestra |
