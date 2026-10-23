# language-preference — Diseño

Los enfoques de fondo están en la [exploración](exploration.md#enfoques-posibles); se eligió I, una preferencia por perfil que elige entre textos fijos. Aquí van las decisiones de implementación de la parte A.

## D1. `clawg_core::preferences`

`crates/clawg-core/src/preferences.rs`, junto a `paths.rs`, porque lo leen el puente, los hooks, `setup`, `doctor` y el CLI (en `clawg-cli`) y el detector y el conteo (en `clawg-core`):

- `Lang { En, Es }`, con `as_str` (`en`, `es`), `parse`, `other` y `name` (`English`, `Spanish`, para las frases en inglés de A). `Serialize`/`Deserialize` en minúsculas.
- `Preferences { language: Option<Lang> }` con `parse(text)` y `to_json()`, puros. `OnDisk` lleva `version` y `language` con `deny_unknown_fields`; un idioma desconocido falla al deserializar.
- `load(base_dir) -> Result<Preferences, PreferencesError>`: ausente es `Preferences::default()`. `save` escribe por `preferences.json.tmp` y `rename`, después de `ensure_private_dir`, como `sharing::save`.
- `language_in(base_dir) -> Option<Lang>`: la lectura que usan todos los que no informan errores. Un error se registra con `log::warn!` y da `None`.
- `paths::preferences_file`.

| Alternativa | Por qué no |
|---|---|
| Un módulo en `clawg-cli` | El detector y el conteo de `clawg-core` necesitan `Lang`, y C hará que el daemon componga prosa por idioma |
| Un archivo inválido es `en` con las frases nuevas | Haría distinto «roto» de «ausente» para el agente sin que nadie lo pida; «sin preferencia» es lo de hoy y `doctor` lo dice |

## D2. El detector

`crates/clawg-core/src/memory/language.rs`, `detect(text) -> Option<Lang>` (`None` es indeciso, `und` en la columna; `column_value(Option<Lang>)`):

1. Quita los bloques ```` ``` ````, los spans entre comillas invertidas y las URL.
2. Separa por espacios y descarta los tokens que tienen `/`, `\`, `_`, `::`, dígitos, un `.` interno o una mayúscula después de la primera letra: rutas, identificadores y nombres de tipos, que en este proyecto son inglés aunque la prosa no lo sea.
3. Recorta la puntuación de los bordes, pasa a minúsculas y cuenta las palabras de dos listas cerradas de palabras funcionales (artículos, preposiciones, conjunciones, pronombres y verbos auxiliares; unas cincuenta por idioma). Las que existen en los dos (`a`, `no`, `me`, `he`) no cuentan. Una palabra con `ñ` o una vocal con tilde, y cada `¿` o `¡`, suma al español.
4. Con menos de `MIN_EVIDENCE` (3) puntos en total, indeciso. Si un idioma tiene al menos el doble que el otro, ese; si no, indeciso.

Es puro y no depende del modelo. Los umbrales se fijan con los tests de LP4 y se comprueban con `rag-eval --lang-detect` sobre el golden (la barra de la propuesta).

| Alternativa | Por qué no |
|---|---|
| `whatlang` o `lingua` | Una dependencia para dos idiomas que una lista de palabras distingue; se reabre si `--lang-detect` no llega a la barra |
| El idioma de la preferencia al guardar | No clasifica las filas anteriores ni ve un save en otro idioma que el pedido (decisión 3.b) |
| Detectar sobre el save entero y repetirlo en cada pieza | La importación y el relleno ven filas, no saves; una pieza tiene al menos 96 tokens, de sobra para decidir, y la regla queda una sola |

## D3. V22

```sql
ALTER TABLE memory ADD COLUMN lang TEXT CHECK (lang IN ('en', 'es', 'und'));
CREATE INDEX IF NOT EXISTS idx_memory_lang ON memory (lang)
    WHERE deleted_at IS NULL AND status IN ('active', 'disputed');
CREATE INDEX IF NOT EXISTS idx_memory_lang_unset ON memory (id) WHERE lang IS NULL;
```

El primero sirve al conteo de LP7, que filtra por idioma y por vida; el segundo al relleno, que busca las filas sin clasificar y queda vacío cuando termina, así que no le cuesta nada a un store ya clasificado. Ningún trigger de `memory` mira `lang` (los de `V8__memory_fts.sql` son de `INSERT`, de `deleted_at` y de `DELETE`), así que clasificar no reescribe `memory_fts`. La vuelta atrás es la de la propuesta: borrar los dos índices antes de `DROP COLUMN`, que SQLite rechaza sobre una columna indexada.

## D4. Escribir el idioma al insertar

Los dos caminos de producción que insertan en `memory`:

- **El save:** `PreparedRow::new` (`crates/clawg-core/src/memory/store_insert.rs`) calcula `lang` de `record.content`, que ya está enmascarado y es el texto de la pieza, y `insert` lo escribe.
- **La importación:** `place` (`crates/clawg-core/src/memory/portability/import.rs`) lo calcula del texto que inserta, ya enmascarado, e `INSERT_SQL` suma la columna. El `lang` del archivo no se lee: el detector da lo mismo sobre el mismo texto, y un archivo no puede declarar algo que el texto no dice.

Cualquier otra fila que entre en `memory` (los tests, el `INSERT` del export de prueba) queda en NULL y la clasifica el relleno.

## D5. El relleno

En `run_maintenance_once_at` (`crates/clawg-core/src/memory/maintenance.rs`), después de la baja de tier: un bucle que lee hasta `LANG_BATCH` (500) filas `WHERE lang IS NULL ORDER BY id`, calcula `detect` fuera de la conexión y escribe `UPDATE memory SET lang = ?2 WHERE id = ?1 AND lang IS NULL` en una transacción por tanda. Termina cuando una lectura no trae filas o una escritura falla (se registra y sigue el resto del mantenimiento). Clasifica también las borradas y las reemplazadas, que pueden volver con `restore` o con el revivir.

Hasta la primera pasada (cinco minutos después de arrancar, `STARTUP_DELAY`), una base que acaba de migrar cuenta sólo lo guardado desde entonces, y la línea del otro idioma no sale. Se acepta: el relleno de una vez al arrancar competiría con el ingest inicial por la única conexión de escritura, que es lo que `STARTUP_DELAY` evita.

## D6. El conteo y la API

`count_other_language(store, lang, project_id, agent_id, reach) -> usize` en `language.rs`, con los predicados de `search.rs` (`memory_project_predicate!`, `reach_predicate!`), así que su alcance no puede apartarse del de la búsqueda:

```sql
SELECT COUNT(DISTINCT COALESCE(
         (SELECT MIN(p.save_id) FROM memory_piece p WHERE p.memory_id = m.id), -m.id))
  FROM memory m
 WHERE m.lang = ?1 AND m.deleted_at IS NULL AND m.status IN ('active', 'disputed')
   AND <proyecto> AND <alcance>
```

Una fila de dos saves cuenta en el de menor id. El conteo es «unas N»: la línea no promete el número exacto de lo que una segunda búsqueda traerá.

`SearchRequest` y `ContextRequest` suman `lang: Option<String>`, que el handler valida con `Lang::parse`: otro valor es un 400 que lista `en` y `es`, como `memory_type`, y no el 422 que daría el extractor de axum con un `enum`. `SearchBody` y `ContextBody` suman `other_language: Option<OtherLanguage>` con `skip_serializing_if`. El conteo corre después de la búsqueda con el mismo proyecto y el mismo alcance; si falla, se registra y el campo no va, como `unreviewed_count` (`api.rs`). En el cliente, `Found` y `Context` lo leen con `#[serde(default)]`.

| Alternativa | Por qué no |
|---|---|
| El daemon lee `preferences.json` | El puente fija el idioma de la sesión (decisión 8) y el archivo puede haber cambiado desde entonces; el pedido dice cuál rige |
| Contar filas en lugar de saves | Un save largo en inglés contaría por cada pieza, y la búsqueda cuenta saves (`distinct_saves`, `search.rs`) |
| Devolver los conteos de todos los idiomas | El puente sólo necesita uno; el campo queda abierto a más idiomas porque nombra el suyo |

## D7. El puente

`ClawgMcp` suma `language: Option<Lang>` y `with_language`, que `crates/clawg-cli/src/mcp/mod.rs` llama con `preferences::language_in(&base_dir)` al arrancar. Con idioma, `search_memory_impl` y `recall_context_impl` mandan `lang`.

`format_search` y `format_recall` (`crates/clawg-cli/src/mcp/tools.rs`) reciben el idioma de la sesión y suman `other_language_line(language, other, shown)` cuando hay idioma, `other.memories > 0` y `shown < FEW_SHOWN` (3). `shown` es lo que el agente ve: los hits que dejó `relevant_hits` en `search_memory` y los `hits` de `Retrieval::Searched` en `recall_context`. El 3 no se midió; la exploración lo propuso y M2, en C, lo calibra. La línea, en inglés en A:

> clawg: 12 memories in this scope are still in English, and a query finds memories best in their own language: if this does not answer, search again with the query in English.

Con una sola memoria, «1 memory in this scope is still in English, and a query finds a memory best in its own language».

## D8. Las frases de idioma del protocolo

`crates/clawg-cli/src/mcp/protocol.rs` pasa a ser el dueño del texto: `PROTOCOL` (el `include_str!` que hoy repiten `server.rs` y `setup.rs`), `opening` y `text(language: Option<Lang>) -> Cow<str>`. Sin idioma devuelve `PROTOCOL` tal cual. Con idioma reemplaza dos frases, buscadas sin distinguir saltos de línea de espacios (el archivo está partido en líneas de 80 columnas y la frase cruza varias):

| Hoy | Con `L` y su otro idioma `O` |
|---|---|
| `Write memories and search queries in English, whatever language the conversation is in: a memory is found best by a query in its own language.` | `Save memories and search in L: a memory is found best by a query in its own language. Reply to the user in L unless they write in another language.` |
| `Write it in English, and search in English too. A memory saved in one language and searched in another is often missed.` | `Write it in L, and search in L too. A memory saved in one language and searched in another is often missed; when a search says memories are still in O, search again with the query in O.` |

La frase nueva de la apertura mide 17 caracteres más que la de hoy: 2.045 de 2.048. El test de 2.048 corre sobre las dos variantes, así que una edición de la apertura que no deje lugar falla ahí. Un test exige que cada frase de hoy aparezca una sola vez en `protocol.md`, así que reescribirla sin tocar `protocol.rs` falla en lugar de dejar el inglés sin aviso.

| Alternativa | Por qué no |
|---|---|
| Partir `protocol.md` para que cada frase quede en sus propias líneas | Cambia el bloque que `setup` escribe sin preferencia (decisión 2) aunque las instrucciones queden iguales |
| Una plantilla con marcadores | Es la 7.b, descartada: los marcadores llegarían al `CLAUDE.md` |
| Un `protocol.es.md` ya en A | Es C; A no traduce texto existente |

## D9. La descripción de `search_memory`

`«; it finds memories written in other words or in another language»` pasa a `«; it finds memories written in other words, but often misses one written in another language than the query»`. Es la única frase que cambia sin preferencia; el contrato se bendice con `CLAWG_BLESS=1` y su diff es esa frase.

## D10. `memory_status`

`format_language(session, now) -> Option<String>` en `tools.rs`, puro, con `now` leído por `language_in` en cada `memory_status`:

| Sesión | Archivo ahora | Línea |
|---|---|---|
| `L` | `L` | `language: L (preferences.json)` |
| `L` | otro, o nada | `language: L for this session; preferences.json now says O, which a new session uses` (o `now sets none`) |
| nada | `L` | `language: none for this session; preferences.json now says L, which a new session uses` |
| nada | nada | ninguna |

## D11. `session-start`

`session_start_text` suma `language: Option<Lang>` y, con idioma, empieza por:

> clawg: the user's language is Spanish. Reply to them in Spanish unless they write in another language, and save and search memories in Spanish.

Va primera porque es una indicación de clawg, no una memoria: el bloque del contexto se presenta como evidencia y no como instrucción. `answer` lee el idioma antes de buscar el daemon; sin cliente, o con el contexto fallido, `session-start` devuelve la línea sola. `respond` usa la misma línea como respuesta cuando vence `HOOK_DEADLINE`. `user-prompt-submit` no la lleva.

## D12. `setup`

`State` suma el idioma del perfil, que `read_state` lee con `language_in`; `plan` sigue siendo puro y `block(eol, protocol)` envuelve `protocol::text(state.language)`. `install_block` ya distingue igual de distinto (`setup.rs`), así que un idioma nuevo da `Rewritten` sin otra rama.

## D13. `doctor`

Dos funciones puras en `doctor/checks.rs`:

- `preferences(state)`, con `state` ausente (sin hallazgos), legible (`ok`) o con su error (`warn` `invalid`, la causa en `message`, el path en `evidence`, sin `safe_next_step`: el archivo es del usuario y la reparación es editarlo o borrarlo).
- `protocol_block(state, profile)`, que sólo corre con preferencia, con el `CLAUDE.md` que `setup::locations` resuelve: `ok`, `stale` (`warn`), `not_installed` (`info`) y `unreadable` (`warn`). Compara el bloque instalado, sin distinguir `\r\n` de `\n`, con el que `setup` escribiría.

Un bloque distinto avisa también cuando lo que cambió es el protocolo de un clawg más nuevo y no el idioma: el remedio es el mismo, `clawg setup claude-code`.

## D14. `clawg preferences`

`Command::Preferences { action: Option<PreferencesAction>, profile }` con `Set { key, value }` y `Unset { key }`, donde `key` es un `ValueEnum` de una sola variante, `language`: la próxima preferencia es otra variante. El comando vive en `crates/clawg-cli/src/preferences.rs`; sus frases son funciones con `match` sobre `Lang`, el molde que B extiende:

| | `en` | `es` |
|---|---|---|
| Sin preferencia | `no language preference: clawg uses English` | — |
| Mostrar | `language: English` | `idioma: español` |
| `set` | `Language: English. Claude Code sessions opened from now on use it; clawg setup claude-code rewrites the protocol block of CLAUDE.md.` | `Idioma: español. Lo usan las sesiones de Claude Code que se abran a partir de ahora; clawg setup claude-code reescribe el bloque del protocolo de CLAUDE.md.` |
| `unset` | `No language preference: clawg uses English. …` (el mismo final) | — |

## D15. `rag-eval --lang-detect`

Un modo de `rag-eval` que carga el golden con el lector de siempre (`golden.rs`), recorre `CorpusRow` con `lang`, llama a `detect` y escribe el JSON de LP15. No abre store ni modelo. El golden expandido (`docs/eval/golden-expanded.json`, fuera de git) trae las 1.280 filas etiquetadas.
