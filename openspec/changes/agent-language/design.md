# agent-language — Diseño

## D1. El protocolo en español es un archivo, no una plantilla

**Elegido:** `protocol.es.md` junto a `protocol.md`, compilado con `include_str!`, y `protocol::text(Some(Lang::Es))` lo devuelve sin tocarlo.

**Alternativas.**
- *Reescribir frases del archivo inglés*, como hace 78 con las dos reglas de idioma (`find_spaced`). Sirve para dos frases; para 160 líneas deja una lista de anclas que se rompe con cualquier cambio de redacción del inglés.
- *Una plantilla con marcadores y una sola fuente* (7.b de la exploración). El español no se escribe palabra por palabra como el inglés y la apertura tiene que caber en 2.048 caracteres con un texto más largo: hay que poder mover frases entre la apertura y las secciones.

**Costo:** dos textos que mantener. Lo controlan tres tests: las mismas secciones `##`, el mismo conjunto de textos entre comillas invertidas, y que cada nombre entre comillas sea una herramienta, un argumento o un tipo que existe (el mismo test que ya corre sobre el inglés).

Con `en`, `text` sigue reescribiendo las dos reglas de `protocol.md` como en 78: nadie que eligió inglés recibe otro texto.

## D2. Las herramientas: una tabla aplicada al router

**Elegido:** `tools_es.rs` tiene `DESCRIPTIONS: &[(&str, Option<&str>, &str)]`, herramienta, argumento (`None` para la herramienta) y texto. `ClawgMcp::with_language(Some(Lang::Es))` recorre `tool_router.map` y reemplaza `attr.description` y la `description` de cada propiedad de `attr.input_schema`.

**Alternativas.**
- *Dos routers*, uno por idioma, con descripciones duplicadas en los atributos `#[tool]`: el macro de rmcp une nombre, descripción y función, así que duplicaría las 12 funciones.
- *Traducir en `list_tools`* sobrescribiendo el handler: la lista es la del router, y `the_agent_contract_matches_its_file` ya lee `tool_router.list_all()`; cambiar el router una vez deja un solo camino para el cliente y para los tests.
- *Un JSON con las traducciones*: necesita parsearse al arrancar y un error de forma aparece en tiempo de ejecución, no al compilar.

Los esquemas de los argumentos tienen la `description` directamente en cada propiedad (no hay `$defs`), así que el reemplazo es un mapa sobre `properties`.

## D3. Las notas del contexto siguen al pedido

**Elegido:** `build_memory_prompt` y `build_session_prompt` reciben un `Lang`, que el daemon saca del `lang` del pedido (`check_lang`, ya validado en 78) y, sin él, `Lang::En`.

**Alternativa:** que el daemon lea `preferences.json` en cada pedido, como lee `sharing.json`. Las notas cambiarían de idioma en una sesión abierta en otro (8.a de 78 fija el idioma por sesión), y una llamada a la API sin puente recibiría español sin pedirlo.

`session-start` no tiene sesión: lee la preferencia en cada evento y manda su `lang`.

## D4. Las frases del puente: un `match` por frase

Como en 79. Las funciones de `tools.rs` reciben `Lang` (las que ya recibían `Option<Lang>` para la línea del otro idioma la usan también para su prosa); `ClawgMcp::speaks()` es el idioma de la sesión, `Lang::En` sin preferencia.

## D5. Lo que el agente cita del daemon

Las frases que el daemon devuelve en un error (`ApiError::bad_request`, `not_found`, `conflict`) y las de errores tipados quedan en inglés: las leen también los clientes de la API y el CLI, y mezclan códigos con texto. El puente las pone dentro de su propia frase («no se guardó: {causa}»), como 79 hace con la causa técnica.
