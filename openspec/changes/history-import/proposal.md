# history-import — Propuesta

| Objetivo | Tamaño | Depende de |
|---|---|---|
| 1, 2 | L | nada pendiente: memory-portability (43), agent-authorship (76), project-reassign (77) y language-preference (78) están archivados, y también 42, 24, 27, 19, 28, 41, 53, 54 y 63 |

## Problema

Lo que el usuario guardó en otra herramienta de memoria no llega a clawg: no aparece en `search_memory` ni en `recall_context`, y la síntesis arranca sin material. El export de engram que se midió trae 3.768 notas en 17 tipos, el 74 % con clave de tema ([exploración](exploration.md#premisas-que-se-verificaron); la segunda medición, del 2026-10-23, en [Idioma](#idioma-medido)).

El import de 43 no alcanza para traerlas:

- **No parte.** Cada línea `memory` es una fila con un vector (`place`, `crates/clawg-core/src/memory/portability/import.rs:283`; `embedding_text(&r.context, &r.content)`, `import.rs:584`), y el 55 % de las notas pasa una pieza: quedarían representadas por sus primeros 512 tokens. Partir pide el tokenizer, que sólo tiene el daemon (`split_save`, `crates/clawg-core/src/memory/chunker/chat.rs:114`; la celda `MemoryRuntime::tokenizer`, `crates/clawg-core/src/runtime.rs:150`).
- **Deja dos vivas con una clave** cuando las dos vienen del mismo archivo y agente: `place` no toma como titular una fila que este import insertó para el mismo agente (`import.rs:307-311`), una regla pensada para las piezas de un save.
- **Duplica si cambia el proyecto:** una fila está presente sólo si coinciden agente, proyecto, hash y fecha (`PRESENT_SQL`, `import.rs:237`), así que volver a importar con otro mapa la inserta de nuevo.
- **No traduce:** 17 tipos a 7, proyectos que vienen como nombre (ninguna sesión trae directorio) o no vienen (43 %), título y contenido separados, la fecha del texto en `updated_at`, 6 claves que clawg no acepta, y las relaciones entre notas de la `0.2.0`.
- **No deja rastro del proyecto de origen.** project-reassign (77) mueve en bloque lo que entra sin proyecto, pero para elegirlo por el nombre que traía cada nota ese nombre tiene que estar en el store, y hoy ninguna columna lo guarda ni ningún filtro lo alcanza (`MoveFilter`, `crates/clawg-core/src/memory/relocate.rs:465`; `ListParams`, `crates/clawg-core/src/memory/admin.rs:887`).

## Decisiones

- **Todo entra, sin pérdidas.** Toda nota se guarda, con los artefactos de SDD, y nada se filtra por tipo, clave ni proyecto. Cada campo de la fuente con un lugar en clawg llega a ese lugar, y el reporte cuenta por su nombre los que no lo tienen, también un campo que la fuente sume después y el adaptador no conozca: aparece en el reporte en vez de perderse sin aviso.
- **El export entero:** el comando y las docs piden `engram export --all`, porque sin él las versiones nuevas de la fuente exportan sólo el proyecto del directorio en que se corre. El import no adivina el alcance, que el archivo no dice, y no le hace falta: un export vacío se importa sin error y sin filas, y uno parcial se ve en las notas por proyecto de la fuente del reporte, que `--dry-run` muestra antes de escribir.
- **Independiente de la fuente.** Un adaptador por herramienta lleva su export a un modelo común de nota importada, y la traducción a filas de clawg es una sola. Este cambio escribe el adaptador de engram; cualquier otra herramienta es otro adaptador sobre el mismo modelo.
- **Se traduce y se parte en el daemon**, por el camino de 43: el CLI manda `from` en `POST /v1/import`. Cada nota se enmascara entera y se parte como un save, con el mismo tokenizer. Sin tokenizer cargado, también un `--dry-run` responde `503`, porque su reporte contaría otras piezas. Un daemon anterior ignora `from`, no encuentra el encabezado de `clawg-memories` y responde `400` sin escribir.
- **Agente `migrated`** para toda fila importada de otra herramienta, porque no se sabe qué agente escribió cada nota; `--agent` lo cambia. Desde 76 es sólo autoría: cualquier agente lee entera, corrige, borra y juzga una fila de `migrated` (`reader_in`, `crates/clawg-core/src/memory/sharing.rs:85`).
- **Proyecto por un mapa `NOMBRE=RUTA`.** Un nombre sin mapa, y una nota sin proyecto o `personal`, entran sin proyecto (se ven desde todos los proyectos, `crates/clawg-core/src/memory/search.rs:110`) y se acomodan con `clawg projects move` (77).
- **El nombre del proyecto de origen en una columna propia** (V23, `memory.origin_project`), escrita sólo por los importadores, indexada, llevada por el export de `clawg-memories`, y un filtro `--origin-project` en `clawg projects move` y `clawg memories list` (con sus rutas). Es lo que 77 le pidió a este cambio; meterlo en `created_by_ref` haría de un mismo texto libre la clave de reconocimiento y un filtro por `LIKE`.
- **Tipos:** `decision` y `architecture` → `architectural_decision`; `preference` → `user_preference`; `session_summary` → `session_summary`; el resto, y un tipo desconocido, → `semantic_fact`. Los prompts → `chat_message` del usuario. El `summary` de una sesión, cuando lo tiene, → `session_summary`.
- **Texto:** el título en la primera línea y el contenido debajo; sin contenido, el título solo. Una nota sin título ni contenido no tiene qué guardar y se reporta.
- **Claves:** una válida se conserva. Una que clawg no acepta se reescribe a una válida y estable: los caracteres que no acepta pasan a `-`, y si hubo que cambiarla o acortarla lleva al final un hash corto de la original, así dos claves distintas nunca se vuelven una. Si notas de distintos orígenes (proyecto y scope de la fuente) caen en un mismo destino con la misma clave, ninguna oculta a la otra: cada clave del grupo se califica con su origen. Dos notas de un mismo origen con una clave dejan viva la más nueva y la otra `superseded` por ella, con causa `revision`. Una clave que en el destino ya tiene una fila viva que no vino de este import sigue la regla de 43: la nota entra `superseded` por esa fila.
- **Fechas, en UTC:** `created_at` es la fecha del texto (`updated_at` de engram, con o sin fracción de segundo, que se trunca a milisegundos), acotada a la hora de importar y reportada si es posterior. `access_count` es `duplicate_count` (las veces que la fuente vio el mismo texto, que empieza en 1 como un save de clawg), `last_accessed_at` es la última vez que la fuente lo vio, la importancia es 5 (9 si la nota está fijada, `pinned`, la que nunca se degrada), y el tier es el que `compute_tier` (`crates/clawg-core/src/memory/tier.rs:22`) le da a un save de ese tipo con esos accesos.
- **Origen:** `source = 'import'` (marcado `imported`, `crates/clawg-core/src/memory/provenance.rs:40`) y `created_by_ref` con la fuente y el id estable de la nota en ella (`engram:<sync_id>`). Un segundo import reconoce una nota por su origen aunque cambie el mapa: con la misma fecha de texto está presente, no la duplica ni la mueve, y el reporte dice en qué proyecto está.
- **Una nota que la fuente reescribió después de importarla** (engram reescribe en su lugar la nota de una clave) entra como una revisión de la guardada: en el proyecto donde está ahora la guardada, con su clave, y la guardada queda `superseded` por ella con causa `revision`. Si la del archivo es más vieja que la guardada, entra `superseded` por la guardada. El import nunca borra ni trae de vuelta: una nota borrada en la fuente después de importarla, o una que el usuario borró en clawg, queda como está y se reporta.
- **Notas de un mismo origen con el mismo texto** (144 en el export medido) se guardan una vez, en la más vieja, con los accesos de todas, como un re-save; el reporte nombra cada una con la que la absorbió. Así un segundo import con otro mapa las reconoce igual.
- **`0.2.0`:** una nota borrada entra borrada, sin vector, y `undelete` la trae. **Las relaciones entre notas entran como pares de `memory_conflict`**: `pending` queda pendiente; `related`, `compatible` y `not_conflict` (y una relación que la fuente marcó `ignored`) quedan juzgadas `compatible`; `scoped`, juzgada `scoped` con su motivo; `conflicts_with`, escalada al usuario; `supersedes`, juzgada `supersedes` con la otra nota reemplazada por veredicto, salvo que la otra sea una preferencia o una decisión y la relación no la haya marcado el usuario: entonces queda escalada, como pide la regla de los veredictos protegidos (63). La exploración las daba por algo que clawg no modela; el vocabulario de la fuente es casi el de `memory_conflict`. Lo que no tiene lugar (la nota en la que la fuente fundió una borrada, la fecha de revisión, el nombre de herramienta, los prompts borrados, una relación reemplazada por otra o huérfana, y todo campo que el adaptador no lee) se cuenta en el reporte. También se cuentan las notas que la fuente reescribió (`revision_count` mayor que 1; 567 en el export del 2026-10-23), cuyas versiones anteriores no vienen.
- **Idioma:** cada fila lleva su idioma en `memory.lang`, calculado por el detector de 78 sobre su texto, como el import de 43. La medición del 2026-10-23 no pide otra regla ([Idioma](#idioma-medido)).

Cada nota entra con su última fecha. Las versiones anteriores de un tema no vienen en el export: la historia de cada tema en clawg empieza por la versión que trae.

### Idioma medido

El 2026-10-23 se midió el export (`0.1.0`) con un programa temporal que pasó el título y el contenido de cada nota, enmascarados, por `clawg_core::memory::language::detect`, e imprimió sólo conteos; el programa y el archivo se borraron después. Repetida el mismo día sobre el export `0.2.0` del mismo store, cada cuenta por nota dio lo mismo; la de filas es una aproximación y no se repitió con el mismo programa.

| Qué | es | en | und | total |
|---|---|---|---|---|
| Notas con texto | 2.231 (59 %) | 1.376 (37 %) | 160 | 3.767 |
| Notas de 8 palabras o más | 2.231 | 1.376 | 7 | 3.614 |
| Artefactos de SDD | 330 | 401 | 4 | 735 |
| `session_summary` | 331 | 128 | 150 (las de sólo título) | 609 |
| Filas, partiendo en tramos de unos 96 tokens | 15.814 | 11.592 | 8.804 (24 %) | 36.210 |
| Prompts con texto | 6 | 1 | 2 | 9 de 14 |

Ninguna sesión tiene `summary`, ninguna nota trae `tool_name`, `deleted_at`, `review_after` ni `superseded_by_sync_id`, y ninguna está fijada (`pinned`, que en `0.2.0` trae cada nota); no hay relaciones ni prompts borrados.

Lo que dice: el detector decide casi toda nota con texto suficiente (7 indecisas de 3.614), y el 24 % de filas `und` (tramos de código) no le quita nada al aviso de otro idioma, porque `COUNT_SAVES_SQL` (`crates/clawg-core/src/memory/language.rs:19`) cuenta una vez un save largo si alguna de sus filas tiene el idioma. La detección por fila de 78 se queda. Una búsqueda en castellano encuentra el 37 % en inglés sobre todo por la línea que pide repetirla en el otro idioma (`other_language_line`, `crates/clawg-cli/src/mcp/tools.rs:137`); traducir notas al importarlas queda fuera.

## Alcance

1. V23: `memory.origin_project`, con su índice, y un índice de `created_by_ref` para las filas `source = 'import'`; el test que siembra V22 y comprueba que cada fila sigue igual.
2. `origin_project` en el formato `clawg-memories` (versión 1, un campo que un lector anterior ignora): el export lo escribe y el import de 43 lo conserva.
3. El filtro `--origin-project NOMBRE` en `clawg projects move` y `clawg memories list`, y `origin_project` en `POST /v1/memories/move` y `GET /v1/memories`.
4. El modelo común de nota importada, en `clawg-core`, y el adaptador de engram (`0.1.0` y `0.2.0`): una función pura que valida `version` y devuelve las notas, los prompts, los resúmenes de sesión, las relaciones y lo que no se traduce, por campo. Lee una lista `null` como vacía y una fecha con o sin fracción de segundo. Una versión desconocida no escribe nada.
5. La traducción común de notas a filas de clawg, pura salvo el chunker que recibe: tipos, texto, proyecto por el mapa, origen, clave, choques de clave, notas de igual texto, fechas, enmascarado de la nota entera, partición como un save en filas `memory` y líneas `piece`, y relaciones en líneas `conflict`.
6. En el import: el reconocimiento por origen antes de `place` (presente, revisión de la guardada o versión vieja), las piezas de una nota escritas en la misma transacción que la nota, y la regla de 43 para todo lo demás.
7. `POST /v1/import` con `from` (`clawg` por defecto, o `engram`) y `project_map`; `clawg import [--from clawg|engram] FILE [--dry-run] [--agent A] [--project-map NOMBRE=RUTA]...`, donde el CLI resuelve cada `RUTA` como un puente y exige que sea un directorio. `--map-root` vale sólo con `--from clawg` y `--project-map` sólo con otra fuente.
8. El reporte, también en `--dry-run`, en el idioma del comando (CL3): notas por tipo de la fuente y de destino, por proyecto de la fuente con su destino, nombres sin mapa y entradas del mapa sin notas, cuántas filas y piezas, cuántas notas se enmascararon, las claves reescritas y las calificadas, las notas reemplazadas por otra de su origen, las de igual texto, las presentes por proyecto, las revisadas, las fechas acotadas, las borradas, las relaciones guardadas y las contadas, lo que no tiene lugar por campo, las notas que la fuente reescribió y las notas sin texto. Para los dos formatos, las filas insertadas por idioma.
9. Docs: `.claude/CLAUDE.md` (el comando, la API, la columna y el filtro), cómo exportar todo desde la otra herramienta (`engram export --all`) a un archivo fuera del repo, el mapeo, y que `migrated` sólo nombra al autor.

### No-alcance

- **Acomodar filas en su proyecto:** es project-reassign (77), que con este cambio puede elegirlas por `--origin-project`.
- La historia de versiones de cada tema, que el export no trae: cada nota entra con su última fecha.
- Traducir notas al idioma de la preferencia.
- Otras fuentes: cada una es un adaptador más sobre el mismo modelo.
- Escribir en la fuente, y las sesiones de la fuente como tabla propia.
- Seguir una nota que la fuente borró: el import nunca borra.

## Cómo se mide

Tests que fallan hoy (no existe `from`, ni el adaptador, ni la partición en el import, ni la columna), sobre exports sintéticos escritos a mano:

1. Cada tipo de engram cae en su destino, y uno desconocido en `semantic_fact`; el reporte lo dice por tipo.
2. El título queda en la primera línea; una nota sin contenido queda con su título; una sin título ni contenido no escribe nada y se reporta.
3. Una nota larga queda en las mismas piezas, con la misma cabecera, que un save del mismo texto, y un secreto en el corte de dos piezas se enmascara entero.
4. El proyecto sale del mapa; un nombre sin mapa, una nota sin proyecto y una `personal` quedan sin proyecto, y cada fila guarda su nombre de origen en `origin_project`.
5. Una clave válida se conserva; una inválida se reescribe igual en dos imports y distinta de otra inválida; dos notas de distintos orígenes con una clave en un destino quedan vivas las dos; dos de un mismo origen dejan una viva y la otra `superseded` con causa `revision`.
6. `created_at` es `updated_at` en UTC, con o sin fracción de segundo; una fecha posterior a la importación queda acotada y reportada; `access_count` es `duplicate_count`; una nota fijada tiene importancia 9.
7. Toda fila tiene agente (`migrated` o el de `--agent`), `source = 'import'` y su origen en `created_by_ref`.
8. Un segundo import no agrega filas ni accesos, tampoco con otro `--project-map` ni después de mover lo importado con `projects move`, y el reporte dice dónde está cada nota.
9. Una nota reescrita en la fuente entra como revisión de la guardada, en su proyecto actual; una versión más vieja entra reemplazada; una borrada en la fuente no cambia nada.
10. Dos notas de un origen con el mismo texto dan una fila con los accesos de las dos.
11. Una `version` desconocida responde `400` sin escribir ni tomar snapshot, y el export de engram mandado sin `from` también; sin tokenizer, también el `--dry-run` responde `503`.
12. `--dry-run` no escribe nada y su reporte coincide con el del import.
13. En `0.2.0`, una nota borrada entra borrada y sin vector; cada relación entra como el par que le toca, `supersedes` sobre una decisión sin la marca del usuario queda escalada, y lo que no tiene lugar se cuenta.
14. Los prompts entran como `chat_message` del usuario, y el resumen de una sesión como `session_summary`.
15. `projects move --origin-project NOMBRE` y `memories list --origin-project NOMBRE` eligen las filas de ese proyecto de origen; un store sembrado en V22 abre con cada fila igual y `origin_project` en NULL; el export y el import de 43 llevan la columna.
16. Desde el agente del puente, `read_memory` devuelve entera una nota importada larga, y `update_memory` y `delete_memory` la cambian.
17. Un export `0.2.0` sin notas, con `sessions` y `prompts` en `null`, se importa sin error y sin filas.
18. Un campo que el adaptador no conoce, en una nota, un prompt, una sesión o una relación, se cuenta por su nombre en el reporte, y una nota con `revision_count` mayor que 1 se cuenta como reescrita en la fuente.

A mano y con autorización, sobre el perfil `dev`: el export real (`engram export --all`) con `--dry-run`, revisar el reporte, importarlo, buscar una docena de temas conocidos, medir cuánto tarda (unos 15 ms por fila con el modelo real; se estiman 36.200 filas, unos 9 minutos) y si el daemon sigue contestando búsquedas y saves mientras tanto, y acomodar un proyecto de origen con `projects move --origin-project`.

`rag-eval` no cambia: el ranking no se toca.

## Migración y vuelta atrás

V23 agrega una columna nullable y dos índices parciales; ninguna fila cambia al migrar, y abrir un store migrado deja antes el snapshot `pre-migration`. Deshacerla es borrar los dos índices y la columna (SQLite no borra una columna indexada). El formato `clawg-memories` sigue en la versión 1: un lector anterior ignora `origin_project`. El import escribe filas nuevas y sólo reemplaza las versiones que revisa; antes deja el snapshot `pre-import`, y `clawg restore` vuelve a él.
