# docs-index-cost — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 (soporte de medición) | S–M | nada en código: el archivo de resultados de docs ya existe (`DocsResults`, `crates/clawg-eval/src/bin/rag-eval/docs.rs:302`). Las dos corridas de cierre necesitan el modelo real del perfil dev y autorización. 95 `docs-index-speed` lee sus números | ver [`state.yaml`](state.yaml) |

## Problema

`rag-eval --real --docs` tardó 2407,1 s en indexar 727 docs en 17 584 pasajes, en Windows y con un build debug ([verify de docs-search-quality](../../docs-search-quality/verify.md), líneas 3 y 11). No hay otro número. Verificado contra el código de hoy:

- **Cada corrida embebe el corpus entero.** El índice se arma en un directorio nuevo del temporal que se borra al terminar (`docs.rs:627`, `:633-645`; `EvalDocIndex::open`, `crates/clawg-core/src/eval_api.rs:1270-1279`), y en un store vacío ningún doc pregunta por vectores guardados (`crates/clawg-core/src/memory/project_docs/mod.rs:459-464`). docs-search-quality planea unas ocho corridas reales, y sus pasos 2 y 3 no cambian ningún vector: sólo tocan el canal léxico ([spec](../../docs-search-quality/spec.md), DQ6 y DQ7).
- **No se sabe en qué se va el tiempo.** El informe imprime el total (`docs.rs:227-237`) y el archivo sólo `index_seconds` (`docs.rs:313`). El refresco hace leer, enmascarar, partir, buscar vectores guardados, embeber y escribir en un solo bucle sin medir cada parte (`mod.rs:447-482`), y el daemon registra sólo su duración total (`record_refresh`, `crates/clawg-cli/src/daemon/api.rs:3016-3025`). Se registran los tokens con relleno (`embedder.batch_tokens`, `crates/clawg-core/src/memory/embedder.rs:509`) pero no los reales, y la evaluación tira su recorder (`Arc::default()`, `eval_api.rs:569`).
- **Las puertas de 95 leen números que nadie produce**: el relleno por tipo de doc, partir y escribir frente al modelo, debug frente a release y el peso de lo archivado ([exploración de 95](../../docs-index-speed/exploration.md), «Enfoques y su puerta»).

## Objetivo

3, como soporte: que medir el índice de docs diga dónde va el tiempo y no pague dos veces el mismo vector. No cambia ningún vector, ningún ranking ni nada guardado.

## Decisiones

1. **Primero se mide y después se memoiza, las dos cosas en este cambio** (enfoques E y B de la [exploración](exploration.md)). Se descartan:
   - **Un store de evaluación que persiste (A).** `plan_refresh` conserva un doc si no cambian su `content_hash` ni su `model_id` (`mod.rs:124`), así que no ve un cambio del partido, del prefijo ni del texto léxico. El resultado de una corrida dependería de cuál corrió antes, y `--diff` dejaría de comparar commits.
   - **El híbrido con una huella completa (C).** Es código de producción y se mete en la huella de docs-search-quality (DQ8).
2. **Una memoria de vectores para `rag-eval --real --docs`, activa por defecto.**
   - Es un decorador de `Embedder` dentro de `eval_api`, así que el embedder de producción no cambia. De cada pedido de pasajes devuelve lo que la memoria tiene y manda el resto al embedder en una sola llamada, en el orden en que llegó. Consulta la memoria tal como estaba al empezar la corrida y agrega lo nuevo al terminar. Así, una corrida con la memoria vacía embebe en las mismas tandas que producción.
   - **La clave** es el SHA-256 de `model_id`, `dimensions`, los dos prefijos (`embedder.rs:120`, `:141`), el SHA-256 del `model.onnx` (`MODEL_SHA256`, `crates/clawg-core/src/memory/model_download.rs:63`), el rol, el texto exacto con su prefijo y la marca de la máquina: sistema, arquitectura, AVX2, AVX-512F y FMA, como `Cpu` en `crates/clawg-core/tests/e5_golden.rs:28-75`. Un vector de otra máquina nunca se usa, porque ahí los últimos bits cambian (`e5_golden.rs:28-31`). Cualquier cambio de lo que se embebe cambia la clave, así que no hay nada que invalidar a mano.
   - **Dónde vive.** En `docs/eval/cache/`, que git ignora (`.gitignore:32`), resuelto como `default_out_dir` (`crates/clawg-eval/src/bin/rag-eval/cli.rs:291-298`). No guarda texto: sólo claves y vectores. Se acumula sin purga, y se limpia borrando la carpeta, como dice la ayuda. Para este corpus ocupa a lo sumo unos 27,6 MB por contrato: 17 584 × (32 + 384 × 4) bytes.
   - **Cuándo no se usa.** `--no-cache` ni la lee ni la escribe, `--fake` nunca la usa y las preguntas no pasan por ella: son 40 consultas y no hay nada que ganar.
   - **Si no se puede leer.** Se ignora con un aviso y la corrida embebe todo. Nunca se usa un vector a medias.
   - **Alternativas descartadas.** Activarla con una bandera: el ahorro es para quien no sabe que existe. Aceptar vectores de otra máquina con tolerancia: un instrumento de medida no puede mover bits. Guardar sólo las claves que usó la última corrida: es más código para un archivo que se limpia borrando una carpeta.
3. **La medición llega a la evaluación y a una línea del log del daemon, sin series nuevas en `metrics.db`.**
   - El refresco mide cada fase por doc con `Instant` (las fases están en la tabla de abajo) y devuelve esos tiempos aparte de `RefreshReport`, porque sus tests lo comparan por igualdad (`mod.rs:885-938`).
   - El daemon escribe una línea `info` después de `indexed the docs of …`, en la forma del resumen de la importación (`Summary`, `crates/clawg-core/src/memory/portability/timing.rs:184-190`): `docs refresh of {project_id}: done in {s} s: scan {s}, read {s}, mask {s}, split {s}, lookup {s}, embed {s}, write {s}`. `refresh_line` y su test (`api.rs:2989-2994`, `:3157-3170`) no cambian.
   - `rag-eval` cuenta los tokens después de indexar. Toma el texto de cada pasaje escrito, con su prefijo, y lo pasa por el tokenizer del modelo con la ventana del embedder (`window_encodings`, `embedder.rs:240-255`). Calcula el relleno por doc en tandas de `EMBED_BATCH` (`embedder.rs:184`, `:436`, `:259-261`) y lo contrasta con `embedder.batch_tokens`, que lee del recorder que `EvalEmbedder::real` pasa a conservar. Es una serie que ya existe, y la evaluación no la escribe en ningún `metrics.db`.
   - **Alternativas descartadas.** Series nuevas como `embedder.tokens`, `embedder.run`, `embedder.tokenize` o `docs.refresh.phase`: cada una entra al contrato de [`specs/metrics`](../../../specs/metrics/spec.md) (R16, R17), y lo que decide 95 es una medición de evaluación. Agregar las fases a `refresh_line`: cambia una línea documentada que fija un test.
4. **`openspec/changes/archive/` sigue en el corpus.** `rag-eval` reparte por área según la ruta relativa: `archive` si empieza con ese prefijo; si no, `code` para `DocKind::Code` y `docs` para el resto (`crates/clawg-core/src/memory/project_docs/corpus.rs:37-45`). Producción no nombra `archive/`: el diseño elige si el refresco entrega los tiempos de cada doc o recibe un clasificador.
5. **El archivo de resultados gana un bloque `index`** (la tabla de abajo). `index_seconds` y `corpus` no cambian, y `DocsFile` no usa `deny_unknown_fields` (`docs.rs:386-391`), así que `--diff` lee los archivos de antes y los de después. `--diff` sigue sin comparar tiempos (`docs.rs:543-571`). No se agrega `reused`, que en un store nuevo vale siempre 0.
6. **El cambio se cierra con dos corridas autorizadas** de `rag-eval --real --docs` con el modelo del perfil dev, sobre el mismo commit, y las dos van a `verify.md`:
   - **D**, en un build debug con la memoria vacía. La llena y da la línea base del golden ampliado de docs-search-quality (su tarea 5).
   - **R**, en un build release en `target-test` con `--no-cache`, que nunca se publica ni se instala. Separa el costo del build del costo del pipeline.
7. **Lo que depende de la medición le toca a 95.** Abajo figura el campo que lee cada candidato.

### El bloque `index`

| Campo | Qué es |
|---|---|
| `index.build` | `debug` o `release` (`cfg!(debug_assertions)`) |
| `index.machine` | `os`, `arch`, `avx2`, `avx512f`, `fma` y `threads` (los hilos de la sesión, `embedder.rs:536`) |
| `index.phases.{scan,read,mask,split,lookup,embed,write}` | Segundos. `scan`: recorrer, leer y hashear todo, más las huellas y el plan (`mod.rs:432-437`). Por doc: `read` (`:453`), `mask` (`:457`), `split` (`:458`), `lookup`, los vectores guardados (`:459-466`), y `embed` (`:468-475`, con la memoria). `write`: `replace_doc` y `remove_docs` (`:480`, `:483`) |
| `index.memo.{used,from_memo,embedded,seconds}` | Si se leyó la memoria; cuántos pasajes salieron de ella y cuántos se embebieron (entre los dos suman `corpus.passages`); cuánto llevó leerla y escribirla, fuera de `index_seconds` |
| `index.tokens.real` | Los tokens de cada pasaje dentro de la ventana de 512, sin relleno |
| `index.tokens.padded` | Filas × el más largo, en tandas de `EMBED_BATCH` por doc y en su orden: lo que corre el modelo sin memoria |
| `index.tokens.padded_ran` | `embedder.batch_tokens` con `role=passage` de la corrida: lo que corrió de verdad |
| `index.tokens.padded_sorted_in_doc`, `index.tokens.padded_sorted_all` | El mismo cálculo con los pasajes ordenados por largo, dentro de cada doc o en toda la corrida |
| `index.tokens.count_seconds` | Cuánto llevó contar, tokenizando como el embedder: la estimación de lo que cuesta tokenizar dentro de `embed` |
| `index.by_area.{docs,code,archive}` | `docs`, `passages`, `multi_batch_docs` (docs con más de `EMBED_BATCH` pasajes), `phases.{read,mask,split,lookup,embed,write}` y `tokens.{real,padded,multi_batch_real,multi_batch_padded}` |

Con `--fake` no se escribe archivo (`docs.rs:591-600`) y el informe por stdout no muestra tokens. Ningún campo lleva texto ni rutas; el informe por stdout nombra el archivo de la memoria.

## Alcance

- El tiempo de cada fase del refresco (puro: `Duration` y conteos) y la línea `docs refresh of …` del daemon.
- Que `EvalEmbedder::real` conserve su recorder. El conteo de tokens, el relleno calculado y los dos órdenes alternativos, puros y con tests de tabla.
- La memoria: la clave, el formato, la decisión de qué falta, leer y escribir sin dejar nada a medias, el decorador, `--no-cache` y su ayuda.
- El bloque `index`, su informe por stdout y el reparto por área.
- Las specs: `specs/eval` («`rag-eval --docs`»: el bloque, la memoria y cuándo deja de valer) y `specs/project-docs` (la línea del log). `specs/metrics` no cambia.
- Las corridas D y R en `verify.md`.

## No-alcance

- Cambiar lo que se embebe, se parte o se guarda: `plan_refresh`, `docs_contract` y la huella son de docs-search-quality.
- Los candidatos de 95: ordenar por largo, solapar partir y escribir con embeber, `prepare_cached` en `replace_doc`, compartir la tokenización, optimizar las dependencias en dev o evaluar en release, una huella completa para que el store de evaluación persista y sacar `archive/`.
- Series nuevas en `metrics.db` y medir por separado las fases internas del embedder (tokenizar, la sesión, el pooling).
- Que `--diff` compare tiempos, la memoria en los otros modos de `rag-eval` y una bandera para usar otra carpeta de memoria (cada worktree tiene su propio `docs/eval/cache/`).
- Los hilos, la espera activa y el tokenizer (62 y 91), y el banco de rendimiento (74).

## Qué lee 95

Las mediciones de 95 que cambian el orden o el tamaño de las tandas corren con `--no-cache`: con la memoria medirían la memoria.

| Candidato de 95 | Lo que lee de este cambio |
|---|---|
| Puerta 0: ¿hay un problema en release? | `index_seconds` de R (`index.build = release`, `index.memo.used = false`) frente al de D |
| A1: ordenar dentro de un doc | por área, `by_area.{docs,code}.tokens.multi_batch_padded / multi_batch_real` y `multi_batch_real / tokens.real`; `tokens.padded_sorted_in_doc` |
| A2: ordenar entre docs | `tokens.padded / tokens.real`; el ahorro estimado `(tokens.padded − tokens.padded_sorted_all) × phases.embed / tokens.padded` frente a `phases.embed` |
| B: `EMBED_BATCH` | nada directo; `tokens.*` y `phases.embed` sirven de base, y su barrido corre con `--no-cache` |
| Solapar partir y escribir con embeber | `phases.read + mask + split + lookup + write` frente a `phases.embed` |
| `prepare_cached` en `replace_doc` | `phases.write / index_seconds` de R |
| Compartir la tokenización | `tokens.count_seconds` y `phases.split` |
| C: dependencias optimizadas en dev, o evaluar en release | `phases.*` de D frente a R; `(phases.scan + mask + split + lookup + write + tokens.count_seconds) / index_seconds` de D |
| Una huella completa para que el store de evaluación persista | `index_seconds − phases.embed` de D: lo que queda con la memoria llena |
| D: sacar `archive/` | `by_area.archive.phases.embed / phases.embed`, `by_area.archive.tokens.real / tokens.real` y `by_area.archive.docs` |

## Cómo se mide el éxito

Tests que hoy fallan. Usan embedders falsos, no necesitan el modelo y no dejan nada en el temporal:

- Con la memoria llena, un segundo indexado no llama al embedder por ningún pasaje y escribe los mismos bytes de vector.
- Si cambia el prefijo, el modelo, la dimensión, el SHA del modelo, el rol o la marca de la máquina, la memoria no se usa (`from_memo = 0`).
- Si cambia un doc, sólo se embeben sus textos nuevos.
- Una memoria truncada o ilegible se ignora con un aviso y se embebe todo. Con `--no-cache` la carpeta queda byte por byte como estaba.
- El reparto por área, el relleno calculado y los dos órdenes alternativos sobre tablas de largos, y la línea del daemon.
- `--diff` compara un archivo sin bloque `index` con uno que lo tiene, y el bloque no lleva texto ni rutas.

Corridas D y R, con el modelo real del perfil dev y autorizadas:

- En las dos, la suma de `index.phases` queda dentro del 5 % de `index_seconds`, y `tokens.padded == tokens.padded_ran`.
- En D, `memo.embedded == corpus.passages`; en R, `memo.used = false`.
- En `--diff` de D contra R ninguna pregunta se mueve en ninguna de las cuatro métricas.
- `verify.md` anota cada campo de la tabla de 95 y el ahorro esperado de la memoria (`phases.embed / index_seconds` de D). La primera corrida que la lea, la del paso 2 de docs-search-quality, anota `memo.embedded` e `index_seconds`; eso no es una puerta de este cambio.
- La suite y los dos clippy pasan.

## Datos y vuelta atrás

- No cambia nada guardado: ni el esquema, ni el contrato de embeddings, ni `memory.db`, ni `metrics.db`. El índice de docs de producción se escribe igual que antes.
- Lo único nuevo en disco es `docs/eval/cache/`, fuera de git. Para volver atrás se revierten los commits y se borra esa carpeta. Un `rag-eval` anterior lee los archivos que traen el bloque `index`, porque no rechaza campos que no conoce.
- El daemon escribe una línea más en el log, y revertir la quita.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| Una memoria vieja esconde un cambio (el peor error para un instrumento de medida) | La clave cubre el texto exacto, el contrato completo y la máquina, con un test por cada componente |
| Alguien lee el `index_seconds` de una corrida con memoria como si fuera la velocidad del modelo | `index.memo` en el archivo y en el informe |
| El relleno calculado deja de coincidir con el real cuando 95 cambie las tandas | Cada corrida sin memoria compara `tokens.padded` con `tokens.padded_ran` y avisa si difieren |
| Medir cambia lo que se mide | Es un `Instant` por fase y por doc frente a una inferencia; la cota del 5 % lo controla |
| La memoria crece sin purga | El tamaño está documentado y se limpia borrando la carpeta |
| El delta de `specs/eval` se apoya en DQ3 y DQ4 de docs-search-quality, que todavía no se archivaron | El cambio que se archive segundo integra el del otro |
