# revision-eval — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | topic-revisions (archivado) | ver [`state.yaml`](state.yaml) |

## Problema

topic-revisions le dio a una decisión que evoluciona una forma de reemplazar a la anterior (`topic_key`), pero nadie midió con el modelo real cuánto hace falta ni si un agente la usa. Detalle en la [exploración](exploration.md). Lo que sigue se volvió a comprobar contra `develop` (`2773c88`):

1. **Sin clave, la versión vieja puede quedar viva sin marca.** El detector sólo empareja filas de tipos que afirman algo (`crates/clawg-core/src/memory/store_insert.rs:59-63`), a una distancia dentro de `[0,10, 0,42)` (`crates/clawg-core/src/memory/contradiction.rs:37`, `:45`) y si la vieja está entre las cinco más cercanas (`CANDIDATE_POOL`, `:58`). Fuera de eso no queda ningún par, y aun con par una fila `disputed` puntúa igual que una `active` (`crates/clawg-core/src/memory/score.rs:295-301`). No se sabe con qué frecuencia pasa.
2. **Con clave, el reemplazo nunca se midió con cadenas de versiones.** `revise_in` (`crates/clawg-core/src/memory/topic.rs:111`) deja `superseded` lo anterior, que sigue en `memory_vectors` y `memory_fts`: la búsqueda lo saca por estado después del corte de vec0 (`crates/clawg-core/src/memory/search.rs:225-232`, `:247`). El golden set no puede llevar una clave (`deny_unknown_fields`, `crates/clawg-eval/src/bin/rag-eval/golden.rs:39-40`) y `EvalIngest::send` guarda siempre sin ella (`crates/clawg-core/src/eval_api.rs:549`), así que ningún `rag-eval` la vio.
3. **El agente ve la clave sólo por algunas herramientas.** La muestran los hits propios de `search_memory` (`crates/clawg-cli/src/mcp/tools.rs:111-113`), `read_memory` (`:183`) y `list_memories` (`:294-302`); `recall_context` y el contexto del hook `session-start`, no (`format_hit_line`, `crates/clawg-core/src/memory/prompt_builder.rs:691-703`, que usa `render` para todos los bloques, `:515-527`). La verificación de topic-revisions hizo una sola corrida por lado ([verify](../2026-10-18-topic-revisions/verify.md#perfil-dev-con-claude-code-2026-10-18)).
4. **Las herramientas de medición no llevan clave.** `FlowSave` (`crates/clawg-eval/src/bin/conflict-eval/measure.rs:60`) no tiene `topic_key`, así que `conflict-eval --flow` reproduce como saves sueltos lo que en un store fueron revisiones.

Hallazgo al verificar: una preferencia de otro proyecto llega a la búsqueda (`search.rs:92-99`) con su `topic=K`, porque el puente sólo oculta la clave en los hits de otro agente (`tools.rs:111-113`), pero un save bajo esa clave no la revisa, porque el alcance es agente y proyecto (`topic.rs:83-86`). Es determinista y no hace falta medirlo: va aparte (ver al final).

## Decisiones

- **Un binario nuevo, `revision-eval`, en `crates/clawg-eval`**, detrás de la feature `eval` como los otros, con dos modos: `chains` (M1, M2, M4) y `store` (M5). Se descarta extender `conflict-eval --flow`. Ese binario mide el detector contra su compuerta (`measure.rs:202-220`), y M1, M2 y M4 necesitan lo mismo que agent-view: el router del daemon en proceso, `format_search`, `format_context` y el contador de tokens (`crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:82-151`). Juntarlos daría un solo reporte y una sola spec para dos herramientas; por la misma razón `golden-mine` es un binario aparte (`crates/clawg-eval/Cargo.toml:21-23`). Lo único que cambia en `clawg-core` es `eval_api`. Un método nuevo, `EvalIngest::save`, recibe además el proyecto y la clave, y espera el informe del worker: ids, candidatas y reemplazos. `send` ya recibe el agente (`crates/clawg-core/src/eval_api.rs:527`) y queda igual para sus llamadores. También se exponen la unión de pedazos de `read_memory` (`join_pieces`, `crates/clawg-core/src/memory/pieces.rs:61`, hoy `pub(crate)`) y las dos constantes de la banda. El estado de cada fila se lee por el router, que ya lo da.
- **Set de cadenas** en `crates/clawg-eval/data/revision-chains.json`, versión 1, versionado como `conflict-pairs.json` porque es texto escrito para medir. Tiene 40 cadenas: 20 en español y 20 en inglés; 24 de dos versiones y 16 de tres, que dan 56 transiciones; 20 `architectural_decision`, 10 `user_preference` y 10 `semantic_fact`, y al menos 8 cadenas con versiones de más de 480 tokens, que se parten en pedazos. Cada cadena lleva `id`, `lang`, `memory_type`, `topic_key`, `query` y `versions` (`content`; `value`, el texto que distingue a esa versión; y `change`: `value` cambia el valor con la misma redacción, `rewrite` lo reescribe, `extend` suma alcance o motivo). La consulta nombra el tema de la cadena, en su idioma, sin el `value` de ninguna versión, y el parseo lo comprueba: así ninguna versión gana por una palabra que sólo ella tiene. El parseo rechaza un set que no llega a esos mínimos, como `Class::minimum` (`measure.rs:41`). Con 56 transiciones, una tasa de 0,5 tiene un intervalo de Wilson de ±0,13.
- **M1 y M2 (`chains`)** corren tres brazos por `EvalIngest` con el modelo real: sin clave, con clave, y un control que guarda sólo la última versión de cada cadena. Los tres usan los mismos distractores: las oraciones con MUST de `openspec/specs`, guardadas antes como hechos (`main.rs:156-171`). Cada transición sin clave queda en una de tres clases: par escalado, par `pending` o sin marca (y entonces se anota si quedó debajo del piso, sobre el umbral o fuera del pool). Para cada cadena se hace su consulta por el router y se registra la posición de cada versión, si alguna vieja aparece en `search_memory` o en `recall_context` después del corte del puente (`tools.rs:65`, `:79`) y si aparece arriba de la nueva. **M2 pasa** cuando hay una sola versión viva por clave en todas las cadenas, ninguna vieja aparece en ninguna salida y la última queda en la misma posición que en el control en al menos 38 de las 40 cadenas, sin bajar más de un puesto en ninguna. Si no pasa, es un bug, y se arregla en un cambio propio.
- **M3 no corre en este cambio.** Una serie útil son unas 30 corridas de dos sesiones de `claude -p`, del orden de 250 a 500 llamadas al modelo, y su resultado vale sólo para el modelo con que se corre. Queda para un cambio propio, que la corre con un modelo de costo bajo (Sonnet o Haiku, según cuántas corridas haga), fijado con `--model` para toda la serie. Para entonces queda este esbozo:
  - **Escenarios.** Decisiones y preferencias, pedidas en español y en inglés. En la sesión 1, el usuario dice una decisión con su motivo y pide que se recuerde. La sesión 2 es nueva (sin `--resume`) y corre en el mismo directorio: el usuario dice que la decisión cambió y por qué. Ninguna nombra una clave, un id ni una herramienta.
  - **Brazos.** En el brazo A, la sesión 2 tiene todas las herramientas. En el brazo B, no tiene `search_memory`, `read_memory` ni `list_memories`, así que la decisión sólo le llega por el hook o por `recall_context`, que no muestran la clave.
  - **Sesión.** Cada sesión carga sólo lo que instala `clawg setup claude-code` (`--strict-mcp-config`), y nada de la configuración de quien corre la serie.
  - **Desenlace.** Se lee del store al final, no del transcript: una sola versión viva por la clave, una sola por veredicto o borrado (un borrado que revive una versión, `crates/clawg-core/src/memory/revival.rs:85`, cuenta por lo que deja), o dos vivas.
  - **Aislamiento.** Cada corrida usa un directorio de proyecto nuevo y borra sus filas al terminar, porque una preferencia se lee desde cualquier proyecto (`search.rs:92-99`).
- **M4 (en `chains`)** cuenta, con el tokenizador de las líneas de base (`EvalTokenCounter::real`, `eval_api.rs:659`) y sobre las consultas del brazo con clave:
  - lo que ` topic=K` suma hoy a `search_memory`: la misma respuesta, formateada con la clave y sin ella;
  - lo que sumaría a `recall_context`: el prompt reescrito con ` topic=K` en la cabecera de cada hit con clave, donde la pondría `format_hit_line`, contado también en bytes contra el presupuesto;
  - el costo fijo de la clave en cada sesión: su descripción en el schema de `save_memory` (`crates/clawg-cli/src/mcp/agent_contract.json:278-285`) y su párrafo de `protocol.md` (`:69-74`);
  - una línea de aviso de referencia multiplicada por la tasa con que dispararía, que da M5.

  El costo en `recall_context` se mide aunque M3 no corra, para que la decisión tenga ese lado listo cuando M3 se haga.
- **M5 (`store`)** lee un `clawg export --include-deleted` de un store real con `format::parse` (`crates/clawg-core/src/memory/portability/format.rs:176`) y une los pedazos de cada save. Mide tres cosas:
  - **Sin reproducir nada:** cuántos saves de hecho, decisión o preferencia llevan clave desde la primera clave del store; cuántas filas quedaron `superseded` por `revision` y cuántas por `verdict` (`supersede_cause`); y cuántos pares se juzgaron `supersedes`.
  - **Reproduciendo el flujo** por el worker con el modelo real, cada save con el agente, el proyecto y la clave con que se guardó: en cuántos saves sin clave dispararía el aviso. El aviso dispara en un save sin clave, de un tipo que afirma, cuyas candidatas del detector (`related`, pendientes o escaladas, `crates/clawg-core/src/memory/worker.rs:86-87`) nombran una fila de su mismo agente y proyecto, las únicas que una clave podría revisar. Una candidata de otro proyecto se cuenta aparte. No se usa `import`, porque no corre el detector (`crates/clawg-core/src/memory/portability/import.rs:3-5`).
  - **Etiquetando:** se toma, para cada save sin clave, el save anterior más cercano del mismo agente y proyecto, y quien mide marca si el nuevo es una versión del anterior (el mismo tema con otro valor, alcance o motivo, de modo que el anterior ya no vale tal como está escrito). Se etiquetan todos los saves en que dispararía el aviso y una muestra con semilla fija de hasta 50 de los demás.

  El export, la lista de pares y las etiquetas llevan texto del usuario: quedan en el directorio temporal del sistema y se borran al terminar. A `docs/eval/results/` van sólo conteos, tasas e intervalos. El store que se mide es el del perfil `prod`, exportado con el `clawg` instalado, porque una build de debug rechaza ese perfil.

## Umbrales, fijados antes de medir

No se ajustan después de medir (regla de [`specs/eval`](../../../specs/eval/spec.md#criterio-de-salida)). Cada umbral dice cuándo vale abrir un cambio; abrirlo no decide cómo se hace.

| Decisión | Entra con | Vale un cambio si |
|---|---|---|
| Aviso «pasá `topic_key`» en un save sin clave con candidata | M5, M4 | se cumplen tres condiciones: al menos el 10 % de los saves sin clave son revisiones (cota inferior de Wilson ≥ 5 %); al menos el 40 % de los saves en que dispara son revisiones; y el aviso cuesta como máximo 40 tokens donde aparece y 15 de media por save del flujo |
| Búsqueda por clave en `search_memory` | M3, M2 | en el brazo A, al menos el 20 % de las corridas con clave termina sin reuso aunque el agente buscó, y en la mitad o más de ellas la búsqueda llevaba algo con forma de clave |
| Mostrar la clave en `recall_context` | M3, M4 | se cumplen dos condiciones: en menos de la mitad de las corridas en que el agente sólo vio la decisión por `recall_context` o por el hook (todo el brazo B y las del A que no buscaron), el agente reusa la clave; y mostrarla suma como máximo 60 tokens de media por prompt del set, con todas las filas con clave, sin que ningún prompt pase de 8.000 bytes |
| Quitar o simplificar reglas de la clave, como la de la pieza compartida (`SHARED_PIECE_SQL`, `topic.rs:91-95`) | M3, M4 | la sesión 1 pone clave en 4 corridas de 30 o menos (cota superior < 0,30), o, en el brazo A, la revisión por clave se da en menos del 30 % de las corridas con clave (cota superior) |
| history-import (44) mapea a `topic_key` las claves de tema de las notas que importa | M1, M2 | M2 pasa, y en al menos el 25 % de las transiciones sin clave la versión vieja queda viva sin marca o aparece arriba de la nueva |

Las filas de la búsqueda por clave, la clave en `recall_context` y quitar reglas se contestan con M3, que no corre en este cambio. Quedan fijadas acá para cuando corra, y este cambio las deja sin contestar. El «4 de 30» supone 30 corridas: con otra cantidad vale la cota superior < 0,30.

De dónde salen los números:

- **10 % de revisiones y 40 % de aciertos.** Con menos de una revisión cada diez saves, el aviso no se justifica. Si se equivoca más de seis veces de cada diez, el agente aprende a ignorarlo.
- **15 tokens de media por save.** Con 5 a 10 saves por sesión, son menos de 150 tokens por sesión.
- **60 tokens por prompt.** Es el orden de lo que se aceptó por LAST SESSION (+48,7 de media) y queda bajo el 6 % del `recall_context` medio (1.037,3).
- **20 % en la búsqueda por clave.** Por debajo, no se justifica un cambio de ranking que exige `rag-eval --diff` sin movimientos.
- **4 de 30.** Es lo más que deja la cota superior de Wilson bajo 0,30.
- **25 % en history-import.** Es una de cada cuatro versiones importadas que quedaría como una segunda viva.

## Alcance

1. `revision-eval`, con `chains` y `store`. La lógica pura va separada de la E/S y cada pieza con su test primero: el parseo del set, la clasificación de transiciones y de saves, la unión con las etiquetas y la reescritura del prompt para M4. Una corrida `--fake` pasa por `EvalIngest` y por el router, como `a_run_finds_a_stored_row_in_both_outputs_and_cleans_up_after_itself` (`crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:353`). Sólo comprueba la tubería, porque el embedder falso no empareja nada (`crates/clawg-eval/src/bin/conflict-eval/main.rs:290`); las clases y los veredictos se prueban con la lógica pura.
2. `eval_api`: `EvalIngest::save` con agente, proyecto y clave, la unión de pedazos y las constantes de la banda.
3. `revision-chains.json` en `crates/clawg-eval/data/`.
4. Las corridas de M1, M2, M4 y M5, con sus resultados fechados en `docs/eval/results/` y el veredicto de cada decisión que contestan en `verify.md`.
5. Al archivar, una sección de `revision-eval` en `specs/eval`.

## Fuera de alcance

- M3, las sesiones de `claude -p`, por su costo: va en un cambio propio, con el esbozo y los umbrales de arriba.
- Cualquier cambio de comportamiento del producto: el aviso, la búsqueda por clave, la clave en `recall_context` o en el hook, o quitar reglas. Cada uno es un cambio propio, y sólo si su umbral lo pide.
- El hallazgo de la preferencia de otro proyecto: va como cambio chico propio, antes de history-import (44), que decide entre ocultar la clave en ese hit o revisar a través de proyectos (lo segundo cambia el alcance que fija `specs/ingest`).
- La clave en `conflict-eval --flow`.
- Tocar el golden set de `docs/eval/`: no puede llevar claves, y cambiarlo movería el ancla con la que se compara todo cambio.
- Medir con otros clientes además de Claude Code.

## Datos guardados

Ninguno: no hay esquema, migración, estado ni identificador nuevo. M1, M2, M4 y M5 leen el modelo del perfil dev, como `conflict-eval --real` (`data_dir_from_env`, `crates/clawg-eval/src/bin/conflict-eval/main.rs:111`), y escriben sus stores en el directorio temporal del sistema; ningún store de un perfil se abre para escribir. M5 sólo lee el export de `prod`. Para volver atrás alcanza con revertir los commits de `crates/clawg-eval` y de `eval_api.rs`.

## Cómo se mide el éxito

- Cada pieza nueva tiene un test que falla antes de escribirla. La suite y el clippy pasan, y el binario que se publica no cambia (el `cargo clippy --locked -p clawg-cli` de CI).
- `rag-eval --real` y `--agent-view`, contra la última ancla (`docs/eval/results/2026-10-18-real-search-abstention.json`) y contra la agent-view de `config.yaml` (`2026-10-20-real-untrusted-recall-fix-agent-view.json`), no mueven ningún caso ni cambian los tokens. Hace falta aunque el producto no cambie. `save` comparte con `send` el armado del evento, y `send` es lo que ingiere el golden de `--agent-view` (`runner.rs:90-93`). `--real` siembra por `EvalStore::insert` (`crates/clawg-eval/src/bin/rag-eval/seed.rs:63-79`) y confirma que el resto de `eval_api` no se movió.
- M1, M2, M4 y M5 se corrieron con el modelo real, con resultados fechados. `verify.md` contesta contra su umbral el aviso y el mapeo de history-import, y deja anotadas como pendientes de M3 las otras tres decisiones. Lo que pasa su umbral queda en el mapa como cambio propio; lo que no, queda anotado con sus números.

## Riesgos

- **Muestras chicas.** El set tiene 56 transiciones y M5 tantos saves como tenga el store. Por eso los umbrales que más pesan usan cotas de Wilson y no la tasa sola.
- **Tres decisiones quedan abiertas sin M3.** Este cambio no puede contestar si el agente pone y reusa la clave; sólo deja medido lo que cuesta mostrarla.
- **El set lo escribe quien mide**, y puede inclinarse hacia reescrituras cercanas o lejanas. Por eso se reporta por tipo de cambio, y M5 lo contrasta con un store real.
- **Las etiquetas de M5 son un juicio.** La regla queda fijada arriba, antes de ver los pares.
- **Tokens del modelo de embeddings, no del cliente.** Se cuentan como en las líneas de base: sirven para comparar entre sí, no son la cuenta del cliente.
## Quién etiqueta M5

Un agente de costo bajo propone cada etiqueta, en una sesión que el dueño del store autoriza, y el dueño decide cada una. Sólo cuenta lo que decide el dueño: un agente solo metería su error sin medir en el umbral del 40 %. El acuerdo entre los dos queda medido.
