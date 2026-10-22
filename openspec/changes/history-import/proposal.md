# history-import — Propuesta

| Objetivo | Tamaño | Depende de |
|---|---|---|
| 1, 2 | M | memory-portability (43), archivado; agent-authorship (76), project-reassign (77) y la parte A de language-preference (78). 42, 24, 27, 19, 28, 41, 53, 54 y 63 ya están |

## Problema

Lo que el usuario guardó en otra herramienta de memoria no llega a clawg: no aparece en `search_memory` ni en `recall_context`, y la síntesis arranca sin material. El export de engram que se midió trae 3.713 notas en 17 tipos, el 74 % con clave de tema ([exploración](exploration.md#premisas-que-se-verificaron)).

El import de 43 no alcanza para traerlas:

- **No parte.** Cada línea `memory` es una fila con un vector (`crates/clawg-core/src/memory/portability/import.rs:571-580`), y el 55 % de las notas pasa una pieza: quedarían representadas por sus primeros 512 tokens. Partir pide el tokenizer, que sólo tiene el daemon (`split_save`, `crates/clawg-core/src/memory/chunker/chat.rs:114`).
- **Deja dos vivas con una clave** cuando las dos vienen del mismo archivo y agente (`import.rs:304-308`).
- **Duplica si cambia el proyecto:** una fila está presente sólo si coinciden agente, proyecto, hash y fecha (`PRESENT_SQL`, `import.rs:236`), así que volver a importar con otro mapa la inserta de nuevo.
- **No traduce:** 17 tipos a 7, proyectos que vienen como nombre (ninguna sesión trae directorio) o no vienen (43 %), título y contenido separados, la fecha del texto en `updated_at`, 6 claves que clawg no acepta.

## Decisiones

Tomadas sobre las [decisiones abiertas](exploration.md#decisiones-tomadas):

- **Todo entra, sin pérdidas.** Toda nota se guarda, con los artefactos de SDD, y nada se filtra por tipo, clave ni proyecto. Cada campo de la fuente con un lugar en clawg llega a ese lugar.
- **Independiente de la fuente.** Un adaptador por herramienta lleva su export a un modelo común de nota importada, y la traducción a filas de clawg es una sola. Este cambio escribe el adaptador de engram; cualquier otra herramienta es otro adaptador sobre el mismo modelo.
- **Se traduce y se parte en el daemon**, por el camino de 43: el CLI manda `from` en `POST /v1/import`. Así cada nota se enmascara entera y se parte como un save, con el mismo tokenizer. Un daemon anterior ignora `from`, no encuentra el encabezado de `clawg-memories` y responde `400` sin escribir.
- **Agente `migrated`** para toda fila importada de otra herramienta, porque no se sabe qué agente escribió cada nota; `--agent` lo cambia. Es sólo autoría: desde agent-authorship (76) cualquier agente lee entera, corrige, borra y juzga toda fila, sea de quien sea. Hoy no: `read_memory` devuelve sólo las filas del agente que llama (`read_save_core`, `crates/clawg-core/src/memory/pieces.rs:108`), así que una nota larga de `migrated` se vería sólo como un fragmento de 500 caracteres en `search_memory`.
- **Proyecto por un mapa `NOMBRE=RUTA`.** Un nombre sin mapa, y una nota sin proyecto o `personal`, entran sin proyecto (se ven desde todos los proyectos, `crates/clawg-core/src/memory/search.rs:95`) y se acomodan con project-reassign (77).
- **Tipos:** `decision` y `architecture` → `architectural_decision`; `preference` → `user_preference`; `session_summary` → `session_summary`; el resto, y un tipo desconocido, → `semantic_fact`. Los prompts → `chat_message` del usuario.
- **Texto:** el título en la primera línea y el contenido debajo; sin contenido, el título solo. Una nota sin título ni contenido no tiene qué guardar y se reporta.
- **Claves:** una válida se conserva. Una que clawg no acepta se reescribe a una válida y estable: los caracteres que no acepta pasan a `-`, y si hubo que cambiarla o acortarla lleva al final un hash corto de la original, así dos claves distintas nunca se vuelven una. Si dos notas de distintos proyectos o scopes de la fuente caen en un mismo destino con la misma clave, ninguna oculta a la otra: cada clave se califica con su origen. Dos notas de un mismo origen con una clave dejan viva la más nueva y la otra `superseded` por ella, con causa `revision`.
- **Fechas, en UTC:** `created_at` es la fecha del texto (`updated_at` de engram), acotada a la hora de importar y reportada si es posterior. `access_count` es `duplicate_count` (las veces que la fuente vio el mismo texto otra vez, lo que el dedup de clawg cuenta al re-guardar), `last_accessed_at` es la última vez que la fuente lo vio, importancia 5, y el tier es el que `compute_tier` (`crates/clawg-core/src/memory/tier.rs:22`) le da a un save de ese tipo con esos accesos.
- **Origen:** `source = 'import'` (marcado `imported`, `crates/clawg-core/src/memory/provenance.rs:40`) y `created_by_ref` con la fuente y el id de la nota en ella. Un segundo import reconoce una nota por su origen aunque cambie el mapa: no la duplica ni la mueve, y el reporte dice dónde está.
- **`0.2.0`:** una nota borrada entra borrada, sin vector, y `undelete` la trae. Las relaciones entre notas se cuentan en el reporte.
- **Idioma, pendiente de medir:** no se sabe en qué idioma vienen las notas de una fuente: el export medido no trae un campo de idioma, y la medición sólo contó campos. Antes de la especificación se mide con un script que imprime sólo conteos por idioma, nunca texto. Cada fila importada lleva su idioma en `memory.lang`, con el detector de language-preference (78), para que la búsqueda en el idioma de la preferencia sepa cuándo repetir en el otro.

Cada nota entra con su última fecha. Las versiones anteriores de un tema no vienen en el export y no hacen falta: la historia de cada tema en clawg empieza por la versión que trae.

## Alcance

1. El modelo común de nota importada, en `clawg-core`, y el adaptador de engram (`0.1.0` y `0.2.0`): una función pura que valida `version` y devuelve las notas, los prompts y lo que no se traduce. Una versión desconocida no escribe nada.
2. La traducción común de notas a filas de clawg, pura salvo el chunker que recibe: tipos, texto, proyecto por el mapa, clave, choques de clave, fechas, origen, enmascarado de la nota entera y partición como un save en filas `memory` y líneas `piece`. Después sigue el camino de 43.
3. El reconocimiento por origen, antes de `place`: una nota cuyo origen ya está en el store cuenta como presente.
4. `POST /v1/import` con `from` (`clawg` por defecto) y `project_map`; `clawg import --from engram FILE [--dry-run] [--agent A] [--project-map NOMBRE=RUTA]...`, donde el CLI resuelve cada `RUTA` como un puente. `--map-root` y `--project-map` no se mezclan.
5. El reporte, también en `--dry-run`: notas por tipo de la fuente y de destino, por proyecto de la fuente con su destino, cuántas filas y piezas, qué se enmascaró, las claves reescritas y las calificadas por un choque, las reemplazadas por una revisión de su mismo origen, las ya presentes con su proyecto, las fechas acotadas, las borradas, las relaciones contadas y las notas sin texto.
6. Docs: `.claude/CLAUDE.md` (el comando y la API), cómo exportar desde engram fuera del repo, el mapeo, y que `migrated` sólo nombra al autor.

### No-alcance

- **Acomodar filas en su proyecto:** es project-reassign (77); el origen por fila permite hacerlo sin duplicar. **Adoptarlas en un agente** deja de hacer falta con agent-authorship (76).
- La historia de versiones de cada tema, que el export no trae: cada nota entra con su última fecha.
- Las relaciones de `0.2.0`, que clawg no modela.
- Otras fuentes: cada una es un adaptador más sobre el mismo modelo.
- Escribir en la fuente, y las sesiones de la fuente como tabla propia.

## Cómo se mide

Tests que fallan hoy (no existe `from`, ni el adaptador, ni la partición en el import), sobre exports sintéticos escritos a mano:

1. Cada tipo de engram cae en su destino, y uno desconocido en `semantic_fact`, reportado.
2. El título queda en la primera línea; una nota sin contenido queda con su título; una sin título ni contenido no escribe nada y se reporta.
3. Una nota larga queda en las mismas piezas, con la misma cabecera, que un save del mismo texto, y un secreto en el corte de dos piezas se enmascara entero.
4. El proyecto sale del mapa; un nombre sin mapa, una nota sin proyecto y una `personal` quedan sin proyecto.
5. Una clave válida se conserva; una inválida se reescribe igual en dos imports y distinta de otra inválida; dos notas de distintos proyectos de la fuente con una clave en un destino quedan vivas las dos; dos de un mismo origen dejan una viva y la otra `superseded` con causa `revision`.
6. `created_at` es `updated_at` en UTC; una fecha posterior a la importación queda acotada y reportada; `access_count` es `duplicate_count`.
7. Toda fila tiene agente (`migrated` o el de `--agent`), `source = 'import'` y su origen en `created_by_ref`.
8. Un segundo import no agrega filas ni accesos, tampoco con otro `--project-map`, y el reporte dice dónde está cada nota.
9. Una `version` desconocida responde `400` sin escribir ni tomar snapshot, y el export de engram mandado sin `from` también.
10. `--dry-run` no escribe nada y su reporte coincide con el del import.
11. En `0.2.0`, una nota borrada entra borrada y sin vector, y las relaciones se cuentan.
12. Los prompts entran como `chat_message` del usuario.
13. Desde el agente del puente, `read_memory` devuelve entera una nota importada larga, y `update_memory` y `delete_memory` la cambian (con 76).

A mano y con autorización, sobre el perfil `dev`: el export real con `--dry-run`, revisar el reporte, importarlo, buscar una docena de temas conocidos y medir cuánto tarda (unos 15 ms por fila con el modelo real; se estiman 36.000 filas, unos 9 minutos) y si el daemon sigue contestando búsquedas y saves mientras tanto.

`rag-eval` no cambia: el ranking no se toca.

## Migración y vuelta atrás

Sin cambio de esquema: el origen va en `created_by_ref`, que ya existe y que el export de 43 lleva. El import escribe filas nuevas; antes deja el snapshot `pre-import`, y `clawg restore` vuelve a él.
