# revision-eval — Diseño

El alcance y los umbrales están en la [propuesta](proposal.md); los requisitos, en la [especificación](spec.md). Aquí van las decisiones de implementación.

## D1. El binario: núcleo puro y cáscara de E/S

**Elegido:** `crates/clawg-eval/src/bin/revision-eval/`, un `[[bin]]` más en `crates/clawg-eval/Cargo.toml` (`:17-31`), sin dependencias nuevas. Sólo `run.rs` y `main.rs` tocan stores, modelo y archivos; el resto es puro y se prueba sin ellos.

| Módulo | Qué tiene |
|---|---|
| `set.rs` | parseo de `revision-chains.json` (con el `value` de cada versión que pide la spec), mínimos, la consulta sin ningún valor |
| `chains.rs` | clase de cada transición (D3), posiciones por versión, veredicto de M2, cortes por `change`, `lang` y tipo |
| `cost.rs` | M4: hits sin clave, reescritura del prompt (D4), costo fijo, la línea de aviso de referencia |
| `store.rs` | M5: saves desde `Parsed`, conteos sin reproducir, disparo, muestra, archivo de etiquetas, cotas (D5) |
| `stats.rs` | `wilson`, `Rate`, `SplitMix64` |
| `run.rs`, `main.rs` | brazos por `EvalIngest`, router en proceso, reproducción, directorio temporal; argumentos y los códigos de salida de `conflict-eval` (`main.rs:20-23`) |

Lo compartido con los otros binarios se **copia** con sus casos fijados: `wilson` y `Rate` de `conflict-eval/measure.rs:107-135` (36/46, 0/10, 10/10), `SplitMix64` como ya lo copia `rag-eval` (`stats.rs:90-110`), el flujo de distractores (`conflict-eval/main.rs:156-171`, `measure.rs:98-104`) y la sesión del router sin su ingesta (`agent_view/runner.rs:82-115`, `:220-255`, con la espera del cierre que pide Windows). El export se lee con `format::parse`, ya público.

| Alternativa | Por qué no |
|---|---|
| Un target `lib` con `stats` | El crate no tiene lib a propósito (`Cargo.toml:11`). Su único uso real sería reemplazar las dos copias de hoy, que difieren en z (1,96 en `measure.rs:111`, 1,959963985 en `rag-eval/stats.rs:29`), y eso movería intervalos ya publicados, fuera de este cambio |
| `#[path = "../conflict-eval/measure.rs"]` | Trae `Class`, `gate` y el resto sin uso, y `dead_code` es error con `-D warnings` |

## D2. `eval_api`: `save` al lado de `send`

**Elegido:** `send` (`eval_api.rs:525`) conserva firma y comportamiento para sus llamadores (`runner.rs:92`, `conflict-eval/main.rs:209-210`, `:221`). Al lado, detrás de `eval` como todo el módulo (`lib.rs:18-19`):

```rust
pub struct EvalSave<'a> { pub agent_id: &'a str, pub project_id: Option<&'a str>, pub content: &'a str,
                          pub memory_type: &'a str, pub importance: f64, pub topic_key: Option<&'a str> }
pub struct EvalSaved { pub rows: Vec<(i64, bool)>, pub related: Vec<EvalCandidate>, pub replaced: Vec<i64> } // bool: insertada
pub struct EvalCandidate { pub id: i64, pub distance: f64, pub escalated: bool }
impl EvalIngest { pub async fn save(&self, save: EvalSave<'_>) -> Result<EvalSaved, EvalError> }
pub async fn row_distances(rt: &MemoryRuntime, from: &[i64], to: &[i64]) -> Result<Vec<(i64, i64, f64)>, EvalError>;
pub fn join_pieces(pieces: &[&str]) -> String;   pub fn is_claim_type(memory_type: &str) -> bool;
pub const NEAR_DUPLICATE_FLOOR: f32;             pub const CONTRADICTION_THRESHOLD: f32;
```

`save` valida tipo y clave (`validate_topic_key`, `topic.rs:33`) antes de encolar, como el daemon antes de la outbox, y espera el `IngestReport` por `reply` (`worker.rs:62`, `:406-408`): el mismo informe con que contesta un save del daemon. Los dos métodos arman el `MemoryEvent` en una función privada, con `user_stated: true` como hoy (`eval_api.rs:552`). `row_distances` usa `vec_distance_l2` sobre `memory_vectors`, que da los mismos valores que el KNN (`search.rs:1008-1012`). `join_pieces` e `is_claim_type` delegan (`pieces.rs:61`, `store_insert.rs:59`) y las constantes se re-exportan (`contradiction.rs:37`, `:45`) con el test de que no son copias, como `EMBED_MAX_TOKENS` (`eval_api.rs:613`, `:1090`). Estado, `superseded_by` y clave de una fila se leen por el router en proceso: `DaemonClient::list` ya los trae (`client.rs:217`, `:230`, `:264`), de a 100 (`admin.rs:908`) y con las `superseded` (`:900-903`).

| Alternativa | Por qué no |
|---|---|
| Cambiar la firma de `send` | `send` no espera: no da los ids ni los candidatos de cada save, y cambiarla toca la ingesta del golden de `--agent-view` |
| Un lector de filas en `eval_api`, como `conflict_pairs` (`:587`) | Superficie nueva para lo que el router ya da; M2 mide lo que el agente ve |
| Hacer `pub` `join_pieces` (`memory/mod.rs:39`) | Lo abre en el binario que se publica |

## D3. M1: lo que se lee y lo que se deduce

```
par escalado → escalated        par pending → pending       (needs_user ya decidió, contradiction.rs:220)
sin par: algún tipo que no afirma → not_claim_bearing
         alguna distancia en [FLOOR, THRESHOLD) → out_of_pool
         alguna < FLOOR → below_floor;   si no → above_threshold
```

El par sale de `conflict_pairs` (`eval_api.rs:587`), con `new_id` entre las filas de la versión nueva y `old_id` entre las de la anterior, que da `save`; las distancias, de `row_distances` entre todos sus pedazos. `out_of_pool` se deduce por descarte: en el brazo sin clave no hay clave ni `derived_from` que sumen hermanos (`worker.rs:886-888`), el agente es uno y sin proyecto, y la vieja sigue viva; un par dentro de la banda sin fila sólo falta porque la vieja no entró al pool de `CANDIDATE_POOL` más los hermanos (`contradiction.rs:198`), que vec0 corta antes de filtrar estado y agente (`search.rs:224-232`). Las posiciones salen de los ids que imprimen `format_search` (`N. #id`, `tools.rs:115`) y las cabeceras del prompt (`[fecha #id`, `prompt_builder.rs:702`).

**Filas compartidas.** Una versión larga que repite casi todo el texto de la anterior guarda cada pedazo sin cambios como una sola fila que los dos saves tienen: el dedup la absorbe (`worker.rs:811-825`), `memory_piece` admite una fila en dos saves (`PRIMARY KEY (save_id, piece)`) y la revisión no reemplaza una fila del save que revisa (`topic.rs:120`). Esa fila es de la versión más nueva que la tiene (`own_rows`): una versión está viva si alguna de sus filas propias lo está, y las posiciones de E3 atribuyen cada hit a la versión más nueva que tiene su fila. Una transición mide las distancias entre las filas que la nueva no comparte con la anterior, que son las que el detector probó, y las propias de la anterior (`compared_rows`); si la nueva tiene todas las filas de la anterior, la transición es `below_floor`. Un par cuenta con cualquier fila de la anterior, compartida o no: es el que el store guarda entre los dos saves. Comparar «todos los pedazos», como dice E2 al pie de la letra, mide la fila compartida contra sí misma a distancia 0: la corrida `--fake` del set dio 9 `below_floor` falsos y 7 cadenas con dos versiones vivas en el brazo con clave.

**Posiciones y M2.** La posición de una versión en una salida es la del primer hit, desde 1 y en el orden impreso, que tiene una fila propia de ella; `none` si no aparece ninguna. La condición (c) de M2 vale en las dos salidas a la vez: una cadena coincide con el control si su última versión queda en el mismo puesto en `search_memory` y en `recall_context`; baja más de un puesto si en alguna queda dos o más abajo, o no aparece donde el control la muestra. «38 de 40» es «se mueven a lo sumo 2» (`MAX_MOVED`), que con el set de 40 son 38. La vieja queda **arriba** de la nueva, en una transición del brazo sin clave, si en alguna salida aparece antes que ella, o aparece y la nueva no. `unmarked_or_above` cuenta las transiciones `no_mark`, las de arriba o las dos una sola vez: es lo que lee el umbral de history-import.

| Alternativa | Por qué no |
|---|---|
| Recalcular el pool con un KNN propio | Copia el corte de vec0, `trim_to_saves` y el alcance; se desincroniza en silencio |
| Re-embeber los textos en la herramienta | Cada pedazo se embebe detrás de la cabecera de su save (`pieces.rs:16-22`): no es la distancia del detector |

## D4. M4: la reescritura vive en la herramienta

**Elegido:** en `search_memory`, la misma `Found` se formatea con `format_search` (`tools.rs:79`) tal cual y con `topic_key = None` en cada hit. En `recall_context`, cada línea de LAST SESSION, RECENT y LONG-TERM (`render`, `prompt_builder.rs:515-527`) que empieza con `[AAAA-MM-DD #id ` gana ` topic=K` antes de ` by ` o del primer `]`, donde `format_hits` la pone (`tools.rs:113-115`). Un texto guardado nunca abre línea (`indent_continuations`, `:663`), así que no hay falso positivo, y el workspace no se toca. Para «todas las filas con clave», un hit sin clave (un distractor) recibe la clave más larga del set: es la cota de arriba. Se cuentan tokens, bytes y prompts reescritos sobre 8.000 bytes, sin recortar: el recorte es del producto. El costo fijo se lee del schema de `save_memory` en `agent_contract.json` y del párrafo de `protocol.md` que abre con la clave; la línea de aviso es una constante fijada antes de medir, que `store` cuenta (D5).

La línea de aviso fijada antes de medir es `this save has no topic_key: if it revises #123, save it again under a topic_key naming their subject, so the next revision replaces it`, lo que `save_memory` agregaría tras su línea `close to earlier memories: ...` con un id de tres dígitos como los de un store real.

| Alternativa | Por qué no |
|---|---|
| Un parámetro en `format_hit_line` o `render` | Cambia una ruta del producto y obliga a abrir `build_prompt` al binario de eval |

La corrida `--fake` comprueba que la reescritura encuentra cada cabecera con clave de un prompt real del router: un cambio del formato del producto la rompe.

## D5. M5: `store` en dos pasos

```
clawg export --include-deleted --out <tmp>/revision-eval-X/export.jsonl      # clawg instalado, perfil prod
revision-eval store --real --export <tmp>/revision-eval-X/export.jsonl      # → labels.jsonl, replay.json
revision-eval store --labels <tmp>/revision-eval-X/labels.jsonl             # → resultado; borra el directorio
```

- **Aislamiento.** El directorio del export debe estar bajo `std::env::temp_dir()` (como `OutsideTemp`, `runner.rs:83-86`); el store de la reproducción es un subdirectorio que se borra al final del primer paso, y el directorio entero al final del segundo o ante cualquier error. El modelo es el del perfil dev (`data_dir_from_env`; sin él, salida 3, `conflict-eval/main.rs:118-121`). Ningún store de un perfil se abre.
- **Saves y orden.** Las filas se agrupan por sus líneas `piece` y se unen con `join_pieces`; un pedazo que falta corta el texto en tramos, como `read_save_core` (`pieces.rs:145-168`). Orden por `(created_at, id)` del primer pedazo, del que salen agente, proyecto, tipo, importancia y clave. Las filas sin agente no se envían y se cuentan. Ni `derived_from`, ni borrados, ni veredictos se reproducen: el origen no cambia pares, y un disparo cuya candidata ya estaba borrada o reemplazada por veredicto se cuenta aparte, como cota del sesgo.
- **Disparo.** Un save sin clave, de un tipo que afirma, cuyo `related` nombra una fila de su alcance (agente y proyecto, `topic.rs:83-86`): las únicas que una clave podría revisar. Una candidata de otro proyecto (una preferencia, `search.rs:92-99`) se cuenta aparte.
- **Etiquetas.** El par de un save que dispara es su candidata más cercana en el alcance; el de los otros, el save anterior más cercano por `row_distances` entre las filas de su alcance vivas en ese momento (lo dice el `replaced` de cada `save`). Van todos los que disparan y hasta 50 de los demás, elegidos y mezclados con `SplitMix64` y la semilla fija (`--seed` la cambia): el archivo no dice cuál disparó. Un save que sólo reforzó filas (el mismo texto) no es una revisión y no se etiqueta. `labels.jsonl` lleva una línea `{"n", "earlier", "later", "proposed": null, "label": null}` por par, con `proposed` y `label` en `revision` u `other`; `replay.json` guarda `n → disparo, tipo` y los conteos, sin texto. El segundo paso rechaza una etiqueta vacía.
- **Quién etiqueta:** un agente de costo bajo (Sonnet o Haiku) llena `proposed` en una sesión que el dueño del store autoriza, y el dueño decide cada `label`, confirmando o corrigiendo. El segundo paso puntúa sólo con `label`, y el resultado da además el acuerdo entre `proposed` y `label` (un `Rate`): dice si un agente podría etiquetar solo la próxima vez. Un agente solo, no: su error entraría sin medir en la precisión del 40 %. Etiquetar a mano todo es el dato más limpio, pero son horas para unos 60 a 150 pares.
- **Cota de prevalencia.** Los que disparan son un censo: `(k_disparo + n_resto · wilson_lo(k_muestra, n_muestra)) / N`; con 50 o menos en el resto, todos se etiquetan y es Wilson de `k/N`.
- **Lo que el diseño no fijaba.** El directorio de trabajo debe ser un `revision-eval-*` hijo directo del temporal (nunca se borra otro), la población de «los sin clave» son los saves sin clave que no sólo reforzaron y tienen un save anterior vivo de su agente y proyecto, de un tipo que afirma (los únicos en que el aviso puede disparar; los demás se cuentan aparte y el promedio por save sigue siendo sobre todo el flujo), y un archivo de etiquetas sin terminar (`label` vacío o inválido) se rechaza dejando el directorio, porque las horas de etiquetado están en él, mientras que cualquier otro error lo borra; una reproducción `--fake` guarda `fake` en `replay.json` y su puntuación sólo se imprime.

| Alternativa | Por qué no |
|---|---|
| Reproducir con `import` | No corre el detector (`import.rs:3-5`) |
| Los pares del export | Los escribió el detector de cada momento, y las filas de antes de la tabla no tienen |
| Un paso que espere las etiquetas | Queda abierto horas; reproducir otra vez para puntuar cuesta minutos de modelo y supone que sale igual |

## D6. Resultados

`docs/eval/results/{fecha}-real-revision-eval-chains[-{tag}].json` y `…-store[-{tag}].json`, como `rag-eval` (`report.rs:688-694`); un archivo existente no se pisa y `--fake` sólo imprime. Cabecera: `model_id` (`eval_api.rs:461`), versión de clawg y, en `chains`, `version` y sha256 del set (`sha256_hex`, `:795`) y cuántos distractores; en `store`, formato y versión del export y la semilla. Cada tasa es un `Rate` (k, n, tasa, Wilson). `chains`: M1 por clase y razón, por `change`, `lang` y tipo; M2 con cada condición, el veredicto y las cadenas que lo rompen; M4 por herramienta. `store`: los conteos sin reproducir, disparos (en alcance, fuera, con candidata ida), revisiones entre los que disparan y en la muestra, la cota, los tokens de la línea (máximo y media por save del flujo). Cada umbral de la propuesta va con su valor y `met`, y `verify.md` cita esos campos sin recalcular. El resultado de `store` no tiene campos de texto, ids de fila ni claves.

## D7. La compuerta de `rag-eval`

`save` comparte con `send` el armado del evento. Con el golden de `docs/eval/` y el modelo del perfil dev, `rag-eval --real --agent-view` contra `2026-10-20-real-untrusted-recall-fix-agent-view.json` y `--real` contra `2026-10-18-real-search-abstention.json`, los dos con `--diff`, sin ningún caso movido ni token de diferencia. Sólo `--agent-view` pasa por `send` (`runner.rs:90-93`); `--real` siembra por `EvalStore::insert` (`seed.rs:63-79`) y confirma que el resto de `eval_api` no se movió.

## D8. Orden para las tareas

Primero lo barato y determinista: `eval_api` con el embedder falso; `stats`, `set` y el set; `chains` y `cost` puros; `run.rs` con su corrida `--fake`. Después, con el modelo del perfil dev, la compuerta de D7 y `chains --real`. `store` puro con exports escritos en el test (como `EXAMPLE`, `format.rs:289`) y su `--fake`. Al final `store --real`, que necesita el export de `prod`, las etiquetas y `verify.md`.
