# history-import — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 2 | M | memory-portability (43); conviene después de 42, 24, 27 y 19 (ver «Dependencias») | exploración (borrador) |

**Tesis:** el usuario ya tiene meses de historia en otra herramienta local de memoria (engram): decisiones, bugs con su causa, resúmenes de sesión. Esas notas no llegan a clawg, que arranca vacío en proyectos donde ya se trabajó mucho. Un importador que traduzca esa fuente a filas de clawg, por el camino de memory-portability (43), hace que esa historia se busque, se puntúe y se sintetice acá.

No es una sincronización: va en un solo sentido, se corre cuando el usuario lo pide, y repetirlo no duplica.

## Premisas que se verificaron

| Premisa | Resultado |
|---|---|
| La fuente exporta un formato documentado | **Cierta.** `engram help` lista `export [file]` («Export all memories to JSON», por defecto `engram-export.json`) e `import <file>`. `ENGRAM_DATA_DIR` cambia el directorio de datos. |
| Forma del export | **Medida sobre un export local, leyendo sólo nombres de campos y conteos, sin leer ningún contenido.** El nivel superior trae `version` (`"0.1.0"`), `exported_at`, `sessions`, `observations` y `prompts`. Cada sesión tiene `{id, project, directory, started_at}`. Cada observación tiene `{id, sync_id, session_id, type, title, content, project, scope, topic_key, revision_count, duplicate_count, last_seen_at, created_at, updated_at}`, aunque algunas no traen `project`. `created_at` es texto de 19 caracteres, sin `T` ni zona. `scope` toma `personal` o `project`. |
| Volumen y forma de esa historia | **Medido en el mismo export:** 22 sesiones, 3.554 observaciones, 14 prompts y ninguna borrada. Las observaciones usan 17 tipos: `architecture` 1.006, `decision` 555, `session_summary` 552, `bugfix` 543, `discovery` 477, `pattern` 303, `preference` 46 y otros diez con menos de 35 cada uno. 2.658 (75 %) tienen `topic_key`. |
| Muchas notas son largas | **Medido.** El contenido tiene una mediana de 2.030 caracteres, un p90 de 5.749, un p99 de 25.143 y un máximo de 45.372. 1.931 notas (54 %) pasan los ~1.900 caracteres, que son ~480 tokens con la conversión de 4 por token que usa el repo (`crates/clawg-core/src/memory/workspace.rs:17-19`; `CHUNK_MAX_TOKENS`, `crates/clawg-core/src/memory/chunker/mod.rs:15`). Es una estimación, pero más de la mitad se partiría en piezas. |
| El export trae la historia de cada tema | **No como se esperaba** (revisado en revision-recheck, 70): trae sólo la versión actual de cada tema. Por eso importarlo deja una sola versión por tema y ninguna fila `superseded`, y lo que vigila M2 de revision-eval aparece recién con las revisiones posteriores, igual que hoy. Una historia de versiones serviría para medir a volumen M1, M2 y M5 de revision-eval, con etiquetas que no dependen del criterio de nadie. Importarla, de esta herramienta o de otras como Notion, queda como decisión abierta. |
| El camino común de save no sirve para esto | **Cierta.** Ver memory-portability (43), enfoque B. Pierde fechas y accesos, pasa por el detector y por el dedup que refuerza. `created_at` sí se puede fijar desde el evento (`crates/clawg-core/src/memory/store_insert.rs:187`). |
| Los 7 tipos de clawg | `chat_message`, `tool_error`, `code_block`, `session_summary`, `user_preference`, `architectural_decision` y `semantic_fact` (`store_insert.rs:19-38`). Sólo los tres últimos con capacidad de afirmar entran al detector (`store_insert.rs:45-50`). |

## Problema (verificado contra el código)

- **Sin importador, la historia queda afuera.** Lo que el usuario aprendió en otra herramienta no aparece en `search_memory` ni en `recall_context`, y la síntesis (objetivo 5) arranca sin material.
- **Importar mal es peor que no importar.** Con las reglas de hoy, miles de notas largas del mismo proyecto que entran juntas:
  - se parten en piezas que compiten con las filas cortas (el caso de long-save-ranking, 19);
  - abren pares en el detector entre notas del mismo tema, y llegan a ACTIVE CONFLICTS (24);
  - pierden su `topic_key`, porque no hay columna (27);
  - pasan por un enmascarado que, si da un falso positivo, destruye texto (42).
- **La traducción no es obvia:**
  - 17 tipos se mapean a 7;
  - el proyecto viene como un nombre y no como una ruta, y sólo la sesión trae `directory`;
  - `title` y `content` van separados;
  - la fecha no trae zona.

## Lo que ya existe y se reutiliza

- El camino de importación y el formato de 43: el importador traduce, y 43 escribe.
- `ProjectIdentity::resolve` (`crates/clawg-core/src/memory/identity.rs:29`) para pasar de `directory`, el de la sesión de cada observación, a un `project_id`.
- `SAVE_HEAD_TOKENS` (`chunker/mod.rs:23-29`): cada pieza después de la primera se embebe detrás de los primeros 48 tokens del save. Poner el `title` en la primera línea hace que el tema viaje en el vector de cada pieza. Es la misma regla que propone memory-protocol (22).
- `redact` en el worker o en el camino de 43, antes de trocear.
- `topic_key` de topic-revisions (27), si existe.
- El origen `import` de memory-provenance (41).
- `source_session_id` de session-handoff (28): las sesiones de la fuente traen id y directorio.

## Enfoques posibles

### A. Leer el export JSON de la fuente (recomendado)

El usuario corre `engram export <archivo fuera del repo>`, y `clawg import --from engram <archivo>` lo traduce al formato de 43 y lo importa.

- **A favor:** es un contrato que la fuente documenta en su CLI, y no depende de sus tablas internas.
- **En contra:** el formato puede cambiar. Se valida `version` y se rechaza una versión desconocida.

### B. Leer la base SQLite de la fuente directamente

- **En contra:** acopla clawg al esquema interno de otra herramienta, con sus tablas de sincronización y sus FTS. Un cambio allá rompe el importador sin aviso. Se descarta.

### Mapeo (a decidir; propuesta inicial)

| Fuente | Destino |
|---|---|
| `decision`, `architecture` | `architectural_decision` |
| `preference` | `user_preference` |
| `session_summary` | `session_summary`, con `source_session_id` |
| `bugfix`, `bug`, `discovery`, `pattern`, `config`, `feature`, `refactor`, `learning`, `convention` y el resto | `semantic_fact` |
| `title` + `content` | `content` con el título en la primera línea |
| `project` / `session_id → directory` | `project_id` por `ProjectIdentity::resolve(directory)`; sin directorio, según el mapa que pase el usuario o sin proyecto |
| `scope = personal` | Sin proyecto, visible desde todos los proyectos (`crates/clawg-core/src/memory/search.rs:204-207`) |
| `topic_key` | `topic_key` (27) |
| `created_at` | `created_at`, interpretado según la zona que se decida |
| `revision_count`, `duplicate_count` | A decidir: accesos iniciales, o nada |
| `prompts` | No se importan (session-handoff, 28, ya descartó guardar prompts por privacidad y ruido) |

Importancia: la fuente no tiene. Propuesta: la de defecto (5) para todo. Con eso, lo importado no entra al workspace, que pide importancia de 8 o más (`crates/clawg-core/src/memory/workspace.rs:255-263`), ni sube al tier 1 por importancia (`crates/clawg-core/src/memory/tier.rs:29-31`). Las preferencias igual entran al tier 1 por su tipo (`tier.rs:35`).

## Descartado

| Opción | Razón técnica |
|---|---|
| Sincronizar en los dos sentidos | Ya descartado en el mapa. Además, cada herramienta tiene su propio modelo de revisión. |
| Importar los prompts del usuario | Privacidad y ruido. Ver session-handoff (28). |
| Fusionar notas parecidas al importar | Reemplaza originales por un texto que nadie escribió. La síntesis (objetivo 5) agrega conclusiones encima y no borra fuentes. |
| Leer el archivo de export desde el repo | `engram export` sin argumento escribe en el directorio actual. El importador tiene que pedir una ruta y la documentación tiene que decir que se exporte fuera del repo, porque el archivo es toda la historia en texto plano. |

## Dependencias

- **memory-portability (43).** Obligatoria: es el camino de escritura.
- **redaction-precision (42).** Antes: un falso positivo en miles de textos se multiplica y no tiene vuelta atrás.
- **conflict-candidates (24).** Antes, o el import saltea el detector (decisión de 43). Si no, miles de notas del mismo tema abren pares que llegan al usuario.
- **topic-revisions (27).** Antes: el 75 % de las notas trae una clave. Sin la columna, la clave se pierde, y la próxima revisión de esa decisión es una fila nueva y no una revisión.
- **long-save-ranking (19).** Conviene antes: el 54 % de las notas sería un save en varias piezas.
- **session-handoff (28) y memory-provenance (41).** Blandas: sin ellas, el origen y la sesión quedan sin escribir.

## Alcance candidato

1. Un traductor puro de la fuente al formato de 43, con su tabla de mapeo y sus casos de test sobre un export sintético escrito a mano (nunca con datos reales).
2. `clawg import --from engram FILE [--dry-run] [--agent A] [--project-map NOMBRE=RUTA ...]`.
3. Un `--dry-run` con el reporte de cuántas notas por tipo de destino, cuántas sin proyecto resoluble, cuántas piezas, qué tipos de secreto se enmascararían y cuántas se saltean por hash.
4. Docs: cómo exportar fuera del repo, y el mapeo.

### No-alcance

- Otras fuentes. El traductor queda separado para que una segunda sea otro módulo.
- Escribir en la fuente.
- Prompts y sesiones como tabla propia.

## Decisiones abiertas

Son del usuario.

- [ ] **Mapeo de tipos.** ¿La tabla de arriba, o `bugfix` y `pattern` con otro destino?
- [ ] **Proyecto.** ¿Por `directory` de la sesión, por un mapa de nombres que pasa el usuario, o las dos?
- [ ] **Agente destino.** ¿Uno fijo (`--agent`, por ejemplo `claude-code`) o uno propio para lo importado? Cualquiera de los dos, pero nunca vacío: el esquema admite `agent_id` NULL con `scope` `project` o `global` (`crates/clawg-core/migrations/V1__init.sql:31-33`), y desde `aa06ba5` una fila sin autor la lee todo agente con cualquier política ([`specs/sharing`](../../specs/sharing/spec.md) R2), pero ningún agente puede cambiarla, borrarla ni restaurarla, y el detector no la compara con lo que se guarda. Lo importado sin agente quedaría a la vista de todos y fuera del alcance de quien quiera corregirlo. Hoy ningún camino de producción escribe NULL (todo save usa `scope = 'agent'`).
- [ ] **Importancia** de lo importado, y si `revision_count` cuenta como uso.
- [ ] **Zona horaria** de `created_at`. La fuente la escribe con `datetime('now')` de SQLite, que es UTC. Hay que confirmarlo antes de fijarlo. Equivocarse hacia adelante no es sólo una fecha corrida. Lo importado nace con `last_accessed_at = created_at`. Desde `871ba7a` una edad negativa ya no sube el decaimiento sobre 1 (`crates/clawg-core/src/memory/score.rs:283-287`), pero la fila puntúa como recién leída hasta que el reloj la alcanza, porque el flush de accesos conserva el valor futuro (`MAX(last_accessed_at, now)`, `crates/clawg-core/src/memory/access.rs:205`), y RECENT la toma como reciente (`bucket_by_recency`, `crates/clawg-core/src/memory/prompt_builder.rs:886`). Una fecha posterior a la importación se acota a la hora de importar y se reporta, sea cual sea la zona elegida.
- [ ] **Detector**: ¿se saltea para lo importado?
- [ ] **Orden respecto de 19 y 24**: ¿se espera a que cierren, o se importa antes con el detector salteado?

## Cómo se mide

Tests RED primero, sobre un export sintético escrito a mano:

1. Cada tipo de la fuente cae en el destino de la tabla, y un tipo desconocido cae en el de defecto y se reporta.
2. El título queda en la primera línea, y en un texto largo cada pieza lleva el título en su cabecera.
3. El proyecto sale del `directory` de la sesión. Sin directorio, sale del mapa o va sin proyecto, según lo decidido.
4. `topic_key` se conserva si existe 27.
5. Un segundo import no agrega filas ni accesos.
6. Una `version` desconocida se rechaza sin escribir.
7. `--dry-run` no escribe nada y su reporte coincide con lo que después se importa.
8. Una nota con `created_at` posterior al momento de importar queda con la hora de importación y aparece en el reporte; ninguna fila importada queda con `agent_id` NULL.

A mano y con autorización, sobre el perfil `dev`: importar el export real, revisar el reporte y buscar una docena de temas conocidos. Se mide cuánto tarda el re-embebido de todo el volumen con el modelo real. `rag-eval` no aplica al importador. Con 19 y 34 ya cerrados, un corpus con notas importadas puede volverse un subconjunto del golden set versionado de 33, escrito a mano y nunca minado de la base real.

## Riesgos

- **Privacidad del archivo de export.** Contiene toda la historia. Nunca va al repo, y el comando lo recuerda.
- **Ruido.** 1.006 notas de arquitectura compiten con lo que se guarda a propósito en sesión. La importancia de defecto, sin pasar al tier 1, lo contiene. Se mide con la búsqueda real.
- **Proyectos mal resueltos.** Una nota en el proyecto equivocado aparece donde no corresponde. El reporte de `--dry-run` lo muestra antes de escribir.
- **Fechas corridas** si se elige mal la zona.
- **Un formato de la fuente que cambia.** La validación de `version` lo frena.
