# language-preference — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 4 | M–L (la parte A de la [exploración](exploration.md#alcance-candidato)) | nada pendiente: agent-authorship (76) y project-reassign (77) están archivados | ver [`state.yaml`](state.yaml) |

## Problema

El detalle, con cada lugar citado, está en la [exploración](exploration.md#problema-verificado-contra-el-código). En resumen:

1. **El idioma de lo guardado es de la instalación, no de la persona.** El protocolo pide inglés en dos lugares: la apertura, que llega como instrucciones del servidor («Write memories and search queries in English, whatever language the conversation is in», `crates/clawg-cli/src/mcp/protocol.md:16-18`), y la sección «How to write a save» (`protocol.md:81-82`). Lo que rinde no es el inglés sino que consulta y fila estén en el mismo idioma: en `similarity`, R@5 `es>es` 18/20 y `es>en` 6/19, y con cualquiera de los dos lados traducido 19/19 y 18/19 ([english-recall, `verify.md:100-102`, `:188-189`](../2026-10-17-english-recall/verify.md)). Una persona que trabaja en español obliga a su agente a traducir cada consulta, y cuando lo olvida el cruce es `es>en`.
2. **Nada le dice al agente en qué idioma hablarle a la persona.** Ni la apertura, ni el protocolo, ni los hooks (`crates/clawg-cli/src/hook.rs`, `session_start_text`) tienen una frase sobre eso.
3. **La descripción de `search_memory` promete lo contrario de lo medido:** «it finds memories written in other words or in another language» (`crates/clawg-cli/src/mcp/server.rs:1141`). `search_memory` encuentra `es>en` en 11 de 19 casos.
4. **Ninguna fila dice en qué idioma está.** Las migraciones V1 a V21 no tienen esa columna, así que, cuando cambie el idioma, nada puede contar las memorias que quedaron en el anterior.

## Decisiones

Las [decisiones abiertas](exploration.md#decisiones-abiertas) se cierran con la opción recomendada. Esta propuesta cubre la parte A; B y C son los cambios 79 y 80:

1. **La preferencia vive en `<data>/preferences.json` (1.a)**, uno por perfil: `{"version": 1, "language": "es"}`, con `deny_unknown_fields` y escrito por archivo temporal y `rename`, como `sharing.json` (`crates/clawg-core/src/memory/sharing.rs`, `load` y `save`). Se cambia con `clawg preferences`. Los valores salen de un conjunto cerrado (`es`, `en`): ninguna preferencia es texto libre que termine en un prompt.
2. **Sin archivo, lo de hoy (2.a).** Las instrucciones, el contexto, los hooks, `setup`, `doctor` y el CLI dan lo mismo que hoy, salvo la descripción de `search_memory` (ver más abajo). Las frases nuevas (cómo hablarle a la persona, en qué idioma guardar y buscar, el aviso del otro idioma) salen sólo con una preferencia escrita. Un archivo inválido cuenta como «sin preferencia», nunca hace fallar un comando y `doctor` lo nombra.
3. **El idioma de cada fila es una columna detectada (3.a):** `memory.lang` con `en`, `es` o `und`, y NULL para «sin clasificar». La escribe un detector puro, sin dependencias, sobre el texto ya enmascarado de la fila, en los dos caminos que insertan: el save y la importación. El mantenimiento clasifica las filas anteriores sin tocar su contenido ni `last_modified_at`.
4. **El daemon cuenta y el agente repite (4.a).** Cuando el pedido nombra un idioma, `POST /v1/search` y `POST /v1/context` con consulta responden `other_language`: cuántos saves vivos del alcance de la búsqueda, que el lector alcanza, están en el otro idioma. El puente agrega una línea sólo si hay preferencia, el conteo es mayor que cero y la búsqueda trajo poco (ningún hit mostrado, o menos de tres). Con conteo cero la línea no sale, y así se apaga.
5. **Marcadores estables, prosa traducida (5.a):** es de C, que traduce lo que lee el agente; A no traduce ningún texto existente.
6. **Se localiza lo que compone clawg (6.a):** es de B y C.
7. **Un archivo por idioma para los textos largos (7.a):** es de C. A deja el protocolo en inglés y sólo cambia, con una preferencia, sus dos frases de idioma y suma la de cómo hablar. Las instrucciones siguen entrando en 2.048 caracteres en cada variante.
8. **El puente fija el idioma al arrancar (8.a).** Una sesión queda en un idioma. `memory_status` dice el idioma de la sesión y avisa cuando `preferences.json` ya dice otro: se aplica en una sesión nueva.
9. **Un idioma por defecto con salvedad (9.a):** «Reply to the user in Spanish unless they write in another language» (en inglés en A; C la traduce).
10. **Tres cambios (10.a):** A es este; B (79, los textos del CLI) y C (80, los textos que lee el agente) dependen de A, y ya no de 76, que está archivado.

**Una excepción a «sin archivo, lo de hoy»:** la descripción de `search_memory` deja de prometer que encuentra memorias «in another language» para todos los clientes, con preferencia o sin ella. Es la única frase que cambia sin preferencia, porque contradice lo medido y la regla del propio protocolo. `agent_contract.json` se vuelve a bendecir sólo por ella.

## Alcance

1. `clawg_core::preferences`: `Lang` (`En`, `Es`), `Preferences`, `parse` y `to_json` puros, `load`, `save` y `language_in`, y `paths::preferences_file`.
2. `clawg preferences [set language es|en | unset language] [--profile P]`: sin argumentos muestra la preferencia; `set` y `unset` dicen que se aplica a las sesiones nuevas del cliente y que `clawg setup claude-code` reescribe el bloque del `CLAUDE.md`. Sus pocas frases salen en el idioma que fija: son el primer uso del molde que B extiende.
3. El detector `clawg_core::memory::language` (`detect`), puro.
4. V22: `memory.lang` y su índice parcial. Escritura en `store_insert.rs` (el save) y en `portability/import.rs` (la importación); relleno por tandas en el mantenimiento; `lang` en las líneas `memory` del export, que el import vuelve a calcular.
5. `lang` opcional en `POST /v1/search` y `POST /v1/context`; `other_language` en sus respuestas, con el mismo alcance (proyecto y `sharing.json`) que la búsqueda; el cliente lo lee con `#[serde(default)]`.
6. El puente: el idioma leído al arrancar; `lang` en cada búsqueda y contexto; la línea del otro idioma en `search_memory` y `recall_context`; `memory_status` con el idioma; las instrucciones con las frases de idioma de la preferencia; la descripción de `search_memory` corregida y el contrato bendecido.
7. `session-start` dice, con preferencia, en qué idioma hablar, guardar y buscar, también sin daemon.
8. `clawg setup claude-code` escribe el bloque del protocolo con las frases de la preferencia.
9. `clawg doctor`: el chequeo `preferences` cuando el archivo existe (inválido, versión, idioma desconocido), y con preferencia, el chequeo `protocol_block`, que compara el bloque del `CLAUDE.md` con el que escribiría `setup`.
10. `rag-eval --lang-detect`: la matriz de confusión del detector sobre las filas del golden que traen `lang`.
11. Las specs (`preferences` nueva, `storage`, `retrieval`, `mcp`, `hooks`, `setup`, `doctor`, `portability`, `eval`) y `.claude/CLAUDE.md`.

## Fuera de alcance

- **Traducir textos existentes**: el protocolo, las descripciones, el contexto y las respuestas del puente (C, 80), y las salidas del CLI (B, 79). Las frases nuevas de A están en inglés y C las traduce.
- **Traducir lo guardado** (filas, resúmenes, `topic_key`), y un modelo de traducción local.
- **Fundir en el daemon dos consultas** (decisión 4.c) y buscar siempre en los dos idiomas.
- **Las mediciones con el modelo real de la paridad español contra inglés (M1), del store mezclado (M2) y de los tokens por idioma (M3):** van en C, porque C traduce el aviso y los textos que agent-view mide, y medirlos antes obligaría a repetirlas. M5 (un hecho en dos idiomas) queda en la exploración.
- **Más idiomas que `es` y `en`**, otras preferencias que el idioma, corregir a mano el idioma de una fila, y el idioma de `search_project`.
- **El ranking:** no cambia; `rag-eval --diff` no aplica.

## Datos guardados

- **Esquema:** V22 agrega `memory.lang` (TEXT, NULL o `en`, `es`, `und`) y un índice parcial sobre las filas vivas. Ninguna fila cambia al migrar: todas quedan en NULL.
- **Antes de migrar** se escribe el snapshot `pre-migration` (`crates/clawg-core/src/memory/db.rs`).
- **El relleno** cambia sólo `lang`, de NULL a un valor, por tandas cortas en la única conexión de escritura. No toca `content`, `content_hash`, `last_modified_at`, `update_count`, el índice de texto ni los vectores: ningún trigger de `memory` mira esa columna (`V8__memory_fts.sql`).
- **Rollback del esquema:** un binario anterior rechaza la base en V22 sin tocarla (`SchemaNewer`). Se vuelve con `clawg restore` del snapshot `pre-migration` o, para conservar lo guardado después:

  ```sql
  DROP INDEX idx_memory_lang;
  ALTER TABLE memory DROP COLUMN lang;
  DELETE FROM refinery_schema_history WHERE version = 22;
  ```

- **La preferencia** no está en `memory.db`: borrar `preferences.json` (o `clawg preferences unset language`) vuelve a lo de hoy. Ningún snapshot, `restore`, `export` ni `import` la lleva.
- **Export e import:** el export suma `lang` a cada línea `memory`, que un lector anterior ignora; el import lo calcula del texto, así que un export de un clawg anterior, sin el campo, da lo mismo.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- sin `preferences.json`, las instrucciones, un contexto, un hook, el bloque de `setup` y `doctor` dan lo mismo que hoy; con un archivo inválido también, y `doctor` lo dice;
- `preferences.json` rechaza versión, campo e idioma desconocidos; `clawg preferences set|unset` escribe por archivo temporal y respeta `--profile`;
- con `es`, las instrucciones piden guardar y buscar en español y responder en español salvo que la persona escriba en otro idioma, y caben en 2.048 caracteres, igual que con `en`;
- el detector clasifica prosa de cada idioma, mezclas, código, textos cortos e identificadores como `en`, `es` o `und`;
- la base sembrada en V21 abre con cada fila igual y `lang` en NULL, y el relleno la clasifica sin tocar el contenido, el hash ni `last_modified_at`; un save y una importación marcan `lang`; un export sin el campo se importa;
- `other_language` cuenta saves vivos del alcance y no los borrados, los reemplazados ni los de un autor oculto por `sharing.json`; sin `lang` en el pedido no aparece;
- la línea del otro idioma sale con conteo mayor que cero y una búsqueda pobre, y no sale con conteo cero, con una búsqueda que trajo lo suficiente, sin preferencia ni ante un daemon que no manda el campo;
- `session-start` con preferencia dice cómo hablar también sin daemon, en menos de 1.000 bytes;
- `setup` con otro idioma reescribe el bloque y `--dry-run` lo dice; `doctor` con preferencia avisa cuando el bloque instalado no es el que `setup` escribiría;
- `memory_status` dice el idioma de la sesión y avisa cuando el archivo cambió desde el arranque.

Medida sin modelo, con el golden expandido de `docs/eval/` (fuera de git): `rag-eval --lang-detect` acierta en al menos el 95 % de las filas etiquetadas de ocho palabras o más y clasifica en el otro idioma a lo sumo el 1 % (`und` no cuenta como acierto ni como cruce). Si no llega, se ajusta el detector o se reabre la decisión 3.d (una dependencia) antes de archivar.

A mano y con autorización, sobre el perfil `dev` (M6 de la exploración): `clawg preferences set language es`, una sesión nueva de Claude Code, el agente responde y guarda en español, una búsqueda pobre muestra la línea del otro idioma y otra que trae lo suficiente no, y `clawg setup claude-code` reescribe el bloque.
