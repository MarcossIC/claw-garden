# history-import — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 2 | M | memory-portability (43); agent-authorship (76) y project-reassign (77). 42, 24, 27, 19, 28, 41, 53, 54 y 63, que convenía tener antes, ya están archivados | exploración cerrada el 2026-10-22; sigue en la [propuesta](proposal.md) |

**Tesis:** el usuario ya tiene meses de historia en otra herramienta local de memoria (engram): decisiones, bugs con su causa, resúmenes de sesión. Esas notas no llegan a clawg, que arranca vacío en proyectos donde ya se trabajó mucho. Un importador que traduzca esa fuente a filas de clawg, por el camino de memory-portability (43), hace que esa historia se busque, se puntúe y se sintetice acá. El importador no está atado a ninguna herramienta: traduce desde un modelo común, y cada herramienta es un adaptador.

No es una sincronización: va en un solo sentido, se corre cuando el usuario lo pide, y repetirlo no duplica.

## Premisas que se verificaron

El export se midió el 2026-10-22 con un script que imprime sólo nombres de campos y conteos, sin leer ningún texto, título ni clave; el archivo se borró después.

| Premisa | Resultado |
|---|---|
| La fuente exporta un formato documentado | **Cierta.** `engram export [file]` escribe todo en JSON, e `import <file>` lo lee. |
| Hay una sola versión del formato | **Falsa.** El export medido declara `"version": "0.1.0"`. Las versiones más nuevas de engram escriben `"0.2.0"`, que suma a cada observación su borrado, la nota en la que se fundió y una fecha de revisión, y al nivel superior las relaciones entre notas y los prompts borrados. También exporta las notas borradas. Engram lee las dos. |
| Forma del export `0.1.0` | Nivel superior: `version`, `exported_at`, `sessions`, `observations`, `prompts`. Sesión: `{id, project, directory, started_at}`. Observación: `{id, sync_id, session_id, type, title, content, project, scope, topic_key, revision_count, duplicate_count, last_seen_at, created_at, updated_at}`. |
| Volumen | 22 sesiones, 3.713 observaciones y 14 prompts; ninguna borrada. 17 tipos: `architecture` 1.015, `session_summary` 605, `decision` 604, `bugfix` 568, `discovery` 493, `pattern` 307, `preference` 48, `feature` 32, `config` 22 y ocho con 5 o menos. `scope`: 3.706 `project` y 6 `personal`. |
| El proyecto sale del `directory` de la sesión | **Falsa.** Las 22 sesiones tienen `directory` vacío. 2.118 notas traen `project`, con 21 nombres distintos; las otras 1.594 (43 %) no tienen proyecto ni en la nota ni en su sesión. El proyecto sólo viene como nombre. |
| Muchas notas son largas | **Cierta.** Título más contenido: mediana 2.055 caracteres, p90 5.628, p99 24.276, máximo 45.403. 2.036 (55 %) pasan los ~1.900 caracteres de una pieza (480 tokens; `CHUNK_MAX_TOKENS`, `crates/clawg-core/src/memory/chunker/mod.rs:15`). Ninguna pasa los 64 KiB de un save (`MAX_CONTENT_BYTES`, `crates/clawg-cli/src/daemon/api.rs:592`). Partidas como un save, a unos 4 caracteres por token, darían unas 36.000 filas. |
| Artefactos de SDD | 702 notas (19 %) tienen una clave `sdd/…` o `sdd-init/…`, con una mediana de 5.369 caracteres: propuestas, specs, diseños y tareas de otros flujos. 509 son `architecture`. Darían unas 20.000 de las 36.000 filas. |
| Las claves de tema pasan a `topic_key` | **Casi todas.** 2.759 notas (74 %) traen clave. Engram admite cualquier carácter salvo los espacios, hasta 120; clawg pide segmentos de `[a-z0-9._-]` unidos por `/` y hasta 80 (`validate_topic_key`, `crates/clawg-core/src/memory/topic.rs:33`; `MAX_TOPIC_KEY_LEN`, `topic.rs:19`). No pasan 6: 4 por largo y 2 por sus caracteres. Engram tiene una por proyecto y scope (ningún grupo repetido), pero si dos proyectos de engram caen en el mismo destino de clawg, chocan 39 claves. |
| Zona horaria de las fechas | **UTC.** Engram escribe sus fechas en UTC sin zona, en las dos versiones del formato. Las 3.713 traen 19 caracteres, sin `T` ni zona, y ninguna es posterior al momento de medir. |
| `created_at` es la fecha del texto | **No siempre.** Engram reescribe en su lugar una nota que se guarda con la misma clave: 563 tienen `revision_count` mayor que 1, y 585 tienen `updated_at` posterior a `created_at`. El texto exportado es el de `updated_at`. |
| El export trae la historia de cada tema | **No.** Trae sólo la versión actual de cada nota, en las dos versiones del formato. |
| Textos | 154 notas tienen `content` vacío (150 son `session_summary`, que guardan todo en el título), una no tiene ni título ni contenido, y 144 repiten el texto de otra del mismo proyecto. |
| El camino común de save no sirve para esto | **Cierta.** Pierde fechas y accesos, corre el detector y el dedup que refuerza. Ver memory-portability (43), enfoque B. |
| El import de 43 parte las notas largas | **Falsa.** Cada línea `memory` es una fila y un vector (`crates/clawg-core/src/memory/portability/import.rs:571-580`, `embedding_text(&r.context, &r.content)`); las piezas sólo llegan como líneas `piece` que ya trae el archivo. Partir pide el tokenizer (`ChatChunker::new_from_base_dir`, `crates/clawg-core/src/memory/chunker/chat.rs:82`; `split_save`, `chat.rs:114`), que carga el daemon desde el caché del modelo. Una nota de 45.000 caracteres en una sola fila tendría un vector de sus primeros 512 tokens. |
| El import de 43 deja una sola viva por clave | **No dentro de un mismo archivo.** `place` busca las vivas con la clave (`TOPIC_HOLDERS_SQL`, `import.rs:245`), pero no toma como dueña una fila que insertó este import para el mismo agente (`import.rs:304-308`), una regla pensada para las piezas de un save. Dos notas distintas con una clave quedarían vivas las dos, lo que `clawg doctor` marca. |
| Un segundo import con otro mapa de proyectos no duplica | **Falsa con el camino de 43.** Una fila está presente si coinciden agente, proyecto, hash y fecha (`PRESENT_SQL`, `import.rs:236`): la misma nota llevada a otro proyecto se inserta de nuevo. |
| Sin proyecto, una fila se ve sólo donde no hay proyecto | **Falsa.** Una búsqueda de cualquier proyecto incluye las filas con `project_id` NULL (`crates/clawg-core/src/memory/search.rs:95`). |
| El origen `import` existe | **Cierta.** `source = 'import'` se marca `imported` en `memory_status`, ACTIVE CONFLICTS y `synthesize` (`mark`, `crates/clawg-core/src/memory/provenance.rs:40`). Un save pone en `created_by_ref` el agente (`crates/clawg-core/src/memory/store_insert.rs:228`), y el import de 43 lo conserva. |
| Una fila de otro agente se puede corregir | **Falsa.** `update` y `delete` rechazan una fila de otro agente (`OWNED_ELSEWHERE_MEMORY`, `crates/clawg-core/src/memory/admin.rs:308`), y el detector compara un save sólo con las filas de su agente. |
| Una fila de otro agente se puede leer entera | **Falsa.** `read_memory` responde "no existe" a una fila de otro agente o sin autor (`read_save_core`, `crates/clawg-core/src/memory/pieces.rs:108`); el puente siempre manda su agente (`crates/clawg-cli/src/mcp/server.rs:908`). Una nota larga de otro agente sólo se ve como el fragmento de 500 caracteres de `search_memory`. |
| Una fila `agent` necesita agente | **Cierta.** `check_memory` rechaza `scope = 'agent'` sin `agent_id` (`crates/clawg-core/src/memory/portability/format.rs:243`). |

## Problema (verificado contra el código)

- **Sin importador, la historia queda afuera.** Lo que el usuario aprendió en otra herramienta no aparece en `search_memory` ni en `recall_context`, y la síntesis (objetivo 5) arranca sin material.
- **Pasar el export tal cual por el import de 43 lo importa mal:**
  - el 55 % de las notas quedaría como una fila con un vector de su comienzo, el caso que long-save-ranking (19) resolvió para los saves;
  - dos notas con la misma clave en un mismo destino quedarían vivas las dos;
  - volver a importar con otro mapa de proyectos duplicaría.
- **La traducción no es obvia:**
  - 17 tipos se mapean a 7;
  - el proyecto viene como un nombre sin ruta, o no viene;
  - `title` y `content` van separados, y a veces `content` está vacío;
  - la fecha del texto es `updated_at` y no `created_at`;
  - 6 claves no son válidas en clawg.

## Lo que ya existe y se reutiliza

- El camino de 43 (`prepare`, `place`, `write_links`, `import.rs`): snapshot `pre-import`, enmascarado, dedup, re-embebido, `--dry-run` y su reporte ([`specs/portability`](../../specs/portability/spec.md)).
- `split_save` y `SAVE_HEAD_TOKENS` (`chunker/mod.rs:29`): cada pieza después de la primera se embebe detrás de los primeros 48 tokens del save. Con el título en la primera línea, el tema viaja en el vector de cada pieza.
- `redact` antes de partir, como el worker (`crates/clawg-core/src/memory/worker.rs:735`): así ningún corte de pieza parte un secreto.
- `derive_project_id` (`crates/clawg-core/src/memory/identity.rs:11`) para la raíz que nombre el usuario, como hace `--map-root`.
- `compute_tier` (`crates/clawg-core/src/memory/tier.rs:22`) para el tier con que nace cada fila.
- `topic_key` (27), el origen `import` (41) y los veredictos protegidos (63).

## Enfoques posibles

### A. Leer el export JSON de la fuente (elegido)

El usuario corre `engram export <archivo fuera del repo>`, y `clawg import --from engram <archivo>` lo traduce y lo importa por el camino de 43. Es un contrato que la fuente documenta en su CLI y no depende de sus tablas. Se valida `version` y se rechaza una desconocida sin escribir.

Dónde se traduce y se parte:

- **A1. En el daemon (elegido).** El CLI manda `{"from": "engram", "text": …}` a `POST /v1/import` (`ImportRequest`, `api.rs:1644`). El daemon traduce con un adaptador puro de `clawg-core` a un modelo común de notas, enmascara cada nota entera, la parte con su chunker en filas `memory` y líneas `piece`, y sigue por el camino de 43. Las piezas son las que daría un save, con el mismo tokenizer. Un daemon anterior ignora `from` (`ImportRequest` no rechaza campos desconocidos, `api.rs:1643`), no encuentra el encabezado de `clawg-memories` y responde `400` sin escribir.
- **A2. En el CLI, con el tokenizer del caché.** No toca el daemon, pero el CLI pasa a leer el caché del modelo. Sin tokenizer parte por bytes, y un segundo import con tokenizer daría otras piezas con otros hashes.
- **A3. El CLI traduce sin partir, y el import de 43 parte toda fila larga.** Cambia el import de los archivos de clawg: una fila larga de un export que hoy se reconoce como presente dejaría de reconocerse al volver a importarla. Se descarta.

### B. Leer la base SQLite de la fuente directamente

Acopla clawg al esquema interno de otra herramienta. Un cambio allá rompe el importador sin aviso. Se descarta.

## Decisiones tomadas

El 2026-10-22, sobre las decisiones abiertas:

- **Todo entra, sin pérdidas.** Toda nota se guarda, los artefactos de SDD incluidos, y nada se filtra por tipo, clave ni proyecto. Una nota que no se pudiera guardar bien es información perdida.
- **Proyecto:** un mapa `NOMBRE=RUTA` que pasa el usuario. Un nombre sin mapa, y una nota sin proyecto, entran sin proyecto; se acomodan con project-reassign (77).
- **`architecture` → `architectural_decision`.**
- **Agente `migrated`** para todo lo importado de otra herramienta, porque no se puede saber qué agente escribió cada nota. El nombre no depende de la fuente: el importador no está atado a engram.

Las resolvió la evidencia:

- **Zona horaria:** UTC.
- **Detector:** el import de 43 nunca lo corre (R3 de [`specs/portability`](../../specs/portability/spec.md)).
- **Orden respecto de 19, 24 y 27:** los tres están archivados.
- **Dónde se traduce:** A1, porque sólo el daemon parte como un save y enmascara antes de partir.

Lo que siguen de ellas:

- **Sin pérdidas aplicado a cada campo:** una clave que clawg no acepta se reescribe a una válida y estable en vez de quitarse; dos notas de orígenes distintos que caerían en un destino con la misma clave no se ocultan una a la otra; `duplicate_count` (cuántas veces la fuente vio el mismo texto de nuevo) pasa a `access_count`, que es lo que el dedup de clawg cuenta en un re-save; los 14 prompts entran como `chat_message` del usuario; en `0.2.0`, una nota borrada entra borrada, recuperable con `undelete`.
- **Un origen por fila:** `created_by_ref` guarda la fuente y el id de la nota en ella. Un segundo import la reconoce por ese origen aunque cambie el mapa, y no la duplica.
- **Con el agente `migrated`,** hoy el agente de siempre leería esas filas sólo como fragmentos de búsqueda: no las lee enteras, no las corrige, no las borra, no las juzga, sus saves no se comparan con ellas y una clave suya no las revisa. Al revisar la propuesta el 2026-10-22 se lo tomó como un fallo del diseño y no como una restricción de la importación: una nota no debe depender del agente que la guardó. El agente pasa a ser sólo autoría en agent-authorship (76), y acomodar filas en su proyecto es project-reassign (77). Este cambio depende de los dos.
- **Versiones viejas:** que el export traiga sólo la última versión de cada tema no es una pérdida que haya que resolver. Cada nota entra con su última fecha.

## Alcance candidato

1. Un modelo común de nota importada y un adaptador puro de engram (`0.1.0` y `0.2.0`) hacia él, con tests sobre exports sintéticos escritos a mano (nunca con datos reales).
2. La traducción común de ese modelo a filas de clawg: tipos, texto, claves, fechas, origen, choques de clave, enmascarado y partición como un save.
3. `clawg import --from engram FILE [--dry-run] [--agent A] [--project-map NOMBRE=RUTA]...`.
4. El reporte, también en `--dry-run`.
5. Docs: cómo exportar fuera del repo, el mapeo y qué queda sólo para leer.

### No-alcance

- Acomodar filas en su proyecto (77) y la autoría del agente (76).
- La historia de versiones de cada tema, que el export no trae: cada nota entra con su última fecha.
- Las relaciones entre notas de `0.2.0`, que clawg no modela: se cuentan en el reporte.
- Escribir en la fuente, y las sesiones como tabla propia.

## Cómo se mide

Tests RED primero, sobre exports sintéticos escritos a mano; la lista final está en la [propuesta](proposal.md#cómo-se-mide). A mano y con autorización, sobre el perfil `dev`: importar el export real con `--dry-run`, revisar el reporte, importarlo, buscar una docena de temas conocidos y medir cuánto tarda (el import de 43 tarda unos 15 ms por fila re-embebida con el modelo real, unos 9 minutos para 36.000 filas) y si el daemon sigue contestando mientras tanto. `rag-eval` no aplica al importador.

## Riesgos

- **Privacidad del archivo de export.** Contiene toda la historia. Nunca va al repo, y el comando lo recuerda.
- **Volumen y ruido.** Unas 36.000 filas, más de la mitad de artefactos de SDD, y las que no tienen proyecto se ven desde todos los proyectos. La importancia de defecto, sin pasar al tier 1, lo contiene en parte; la búsqueda real lo mide. El agrupamiento del KNN en un store grande es el tema de knn-prefilter (37).
- **Una escritura larga en la única conexión de SQLite del daemon** (read-connections, 57): se mide en el perfil `dev`.
- **Proyectos mal asignados.** El reporte de `--dry-run` los muestra antes de escribir.
- **Un formato de la fuente que cambia.** La validación de `version` lo frena.
