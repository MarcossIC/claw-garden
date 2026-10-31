# docs-index-speed — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 (y el tiempo de medir el índice de docs) | S–M, cada paso detrás de su puerta; puede cerrar sin código | **#94 `docs-index-cost`**: sus corridas D y R y su bloque `index` son lo único que leen las puertas. Cada corrida necesita el modelo real del perfil dev y autorización | ver [`state.yaml`](state.yaml) |

## Problema

`rag-eval --real --docs` tardó 2407,1 s en indexar 727 docs en 17 584 pasajes, en Windows y con un build debug ([verify de docs-search-quality](../docs-search-quality/verify.md), líneas 3 y 11). Verificado contra el código de hoy:

- **El refresco embebe doc por doc y sin ordenar.** Un bucle secuencial lee, enmascara, parte, pide los vectores guardados, embebe y escribe cada doc (`crates/clawg-core/src/memory/project_docs/mod.rs:447-482`). Manda todos los pasajes de un doc en una llamada (`:566-568`), que el embedder corta en tandas de `EMBED_BATCH` = 16 consecutivos (`crates/clawg-core/src/memory/embedder.rs:184`, `:436`) y rellena hasta el más largo (`pad_right`, `:259-261`). La atención crece con textos × tokens² (`:179-181`).
- **Hay un antecedente medido, de otro corpus.** La importación ordena de más tokens a menos (`longest_first`, `crates/clawg-core/src/memory/portability/import.rs:2537`). Eso bajó su fase `embed` un 30,4 %, y las 5000 filas comparadas dieron los mismos bits ([verify de embed-throughput](../archive/2026-10-27-embed-throughput/verify.md), línea 30; [propuesta](../archive/2026-10-27-embed-throughput/proposal.md), línea 16). Ese cambio dejó el refresco de docs como «otro cambio, con su propia medición» ([design](../archive/2026-10-27-embed-throughput/design.md), línea 54). Midió filas de memoria, no pasajes de docs.
- **Lo que falta saber lo mide #94**: el tiempo por fase, los tokens reales contra los rellenados (también en los dos órdenes posibles, sin correrlos), debug contra release y el peso de cada área ([propuesta de #94](../docs-index-cost/proposal.md), «El bloque `index`» y «Qué lee 95»). Esta propuesta fija ahora, antes de ver esos números, qué campo decide cada mejora y con qué umbral.

## Objetivo

3: que indexar un proyecto tarde menos, tanto la primera vez en producción como en cada corrida de la evaluación, sin cambiar qué encuentra `search_project`. Si ninguna puerta abre, el cambio se cierra con el registro de lo que se leyó y sin código.

## Decisiones

1. **`openspec/changes/archive/` sigue en el índice de `search_project` en producción.** Guarda la historia de las decisiones, y 5 respuestas esperadas del golden de docs son copias archivadas (`crates/clawg-eval/golden/project-docs.json:44`, `:68`, `:107`, `:115`, `:128`). Se descartan dos alternativas:
   - Sacarlo por defecto o con un ajuste: es una decisión de producto sobre lo que encuentra el agente, no de velocidad, y producción no puede nombrar una convención de este repo.
   - Sacarlo sólo en la evaluación con `left_out`: mediría otro corpus, sin los duplicados que hacen difícil el ranking.

   Este cambio lee `index.by_area.archive` sólo para explicar los números.
2. **Qué es de este cambio.** Ordenar los pasajes por largo antes de armar las tandas se hace acá y no en 62 embedder-footprint. 62 lo lista como enfoque ([exploración](../embedder-footprint/exploration.md), línea 89), pero deja fuera el tamaño de tanda (línea 138). Además, el refresco de docs es el único camino que arma tandas con largos mezclados. Los hilos de ONNX Runtime, la espera activa y el grupo de `rayon` quedan en 62; el tamaño del tokenizer, en 91.
3. **La puerta 0 lee `index.phases.embed` de R, no `FIRST_INDEX_WAIT`.** La exploración cerraba A si la primera indexación entraba en `FIRST_INDEX_WAIT` (20 s, `crates/clawg-cli/src/daemon/api.rs:3034`). Con 17 584 pasajes en una CPU, eso no decide nada. Al ritmo que midió la importación (65,9 filas/s con 129,3 tokens por fila; verify de embed-throughput, línea 30, y propuesta, línea 11), en 20 s se embeben unos mil pasajes. La puerta pasa a ser el tiempo de embeber en release y la parte del total que ocupa (paso 0).
4. **Bits.**
   - Toda medición que cambia el orden de las tandas corre con `--no-cache`: con la memoria de #94 activa, se mediría la memoria (#94, «Qué lee 95»). C y W no cambian ningún vector y pueden usarla.
   - A1 y A2 cambian qué textos comparten una corrida sin tocar `E5Embedder`, así que hoy `crates/clawg-core/tests/e5_golden.rs` no los ve.
     - Para que los vea, `e5_golden` gana un caso: los 17 textos de «a batch of 17» (`e5_golden.rs:211-218`, `:241`), en una sola llamada y en el orden que les da A1.
     - El caso se escribe con el procedimiento de la fixture (`CLAWG_BLESS=1`, `:13-14`), en la máquina cuya huella nombra la fixture: hoy Windows x86_64 con AVX2, AVX-512F y FMA. El bless escribe la huella de la máquina donde corre (`:257`), así que en otra cambiarían todos los casos.
     - En cada otra CPU donde se mida, el test compara como hoy, con 1e-6 (`:26`, `:47-53`).
     - Además, una prueba ignorada como la de la importación (`import.rs:5579-5596`) cuenta, sobre los pasajes reales de docs, cuántos vectores dan los mismos bits en los dos órdenes, y la diferencia mayor.
   - **Si se mueve un bit, se reescribe la fixture con su procedimiento. Nunca se agranda la tolerancia.**
     - Se acepta sólo si la diferencia mayor es ≤ 1e-6, la cota que `e5_golden` ya acepta entre máquinas, y `rag-eval --real --docs --diff` no mueve ninguna pregunta.
     - Por encima de eso, cambia el contrato de vectores: el paso cae y el tema pasa a 17 model-reembed.
     - Al adoptar el paso se vacía la memoria de #94 borrando `docs/eval/cache/` (su limpieza documentada), porque su clave no incluye cómo se armó la tanda.
     - Por qué reescribir y no tolerar: `e5_golden` es la única comparación exacta que existe. Una tolerancia en la CPU de la fixture lo dejaría ciego a todo lo que se mueva después en esa máquina, como los hilos de 62 o una versión nueva de ONNX Runtime. Reescribir la deja exacta. El bless se detiene para que se revise el diff (`:249-262`), así que git guarda qué vector se movió y cuánto. Además, ninguna fila se compara por bits: el refresco reutiliza los vectores por texto (`reuse_plan`, `mod.rs:498-510`).
   - C no cambia código: si mueve un bit, cae, sin reescribir nada.
5. **Se descartan B (`EMBED_BATCH`), la tokenización compartida y una huella completa** (ver «Descartados»).

## Pasos con puerta

Cada puerta lee campos de #94 tomados de R (release en `target-test`, `--no-cache`) o de D (debug, memoria vacía), de una corrida del mismo commit y la misma `index.machine`, o de la corrida del paso anterior, hecha de la misma forma. Un paso que pasa su puerta se implementa, y queda sólo si su corrida, comparada con la anterior, cumple «se queda si».

Un 10 % está muy por encima del ruido de una sola corrida: import-timing midió 20 s de diferencia entre dos corridas con el mismo trabajo, mientras el `embed` de la importación cambió 361 s (verify de embed-throughput, línea 51). La estimación de ahorro de #94 es lineal en tokens y se queda corta. Con 1,293 veces los tokens rellenados (propuesta de embed-throughput, línea 13), estimaba (1,293 − 1,001) / 1,293 = 22,6 %, y midió 30,4 %.

| Orden | Paso | Campo de #94 | Abre si |
|---|---|---|---|
| — | 0. ¿Embeber pesa en release? | R: `index.phases.embed` y `index.phases.embed / index_seconds` | ≥ 120 s **y** ≥ 0,5 |
| 1 | C. Dependencias optimizadas en dev, o evaluar en release | D y R: `(phases.scan + mask + split + lookup + write + tokens.count_seconds) / index_seconds` | ≥ 0,25 en D **y** esa suma en R ≤ la mitad de la de D |
| 2 | W. `prepare_cached` en `replace_doc` | R: `phases.write / index_seconds` | ≥ 0,10 |
| 3 | A1. Ordenar por tokens dentro de un doc | R: `(tokens.padded − tokens.padded_sorted_in_doc) / tokens.padded` | ≥ 0,10 **y** ese ahorro × `phases.embed` ≥ 2 × `tokens.count_seconds` |
| 4 | S. Solapar leer, partir y escribir un doc con embeber otro | La corrida de W (o R): `(phases.read + mask + split + lookup + write) / index_seconds` | ≥ 0,15 |
| 5 | A2. Ordenar entre docs con una cola acotada | R: `(tokens.padded_sorted_in_doc − tokens.padded_sorted_all) / tokens.padded` | ≥ 0,15 |
| 6 | E. Solape propio de docs de 32 tokens | La puerta 0 en la última corrida; después, la corrida del experimento | Puerta 0 abierta, y en el experimento `tokens.real` y `phases.embed` bajan ≥ 10 % |

Van de menor a mayor riesgo. El paso 0 abre A1, A2 y E. Si `embed` es menos de la mitad del total, S pasa antes de A1 y el paso 0 se vuelve a leer en la corrida que sigue a W y S.

**0.** Con menos de 120 s, el 10 % que pide A1 ahorra menos de 12 s en la primera indexación de este repo, menos que `FIRST_INDEX_WAIT`. No cuesta nada: lee R.

**C.**
- **Por qué ese umbral:** con la memoria llena, esa suma es lo que todavía cuesta cada corrida debug de docs. Un cuarto de los 2407 s de hoy son unos 10 min, y docs-search-quality planea unas ocho corridas. Si release no la reduce a la mitad, el costo no viene del nivel de optimización.
- **Variante a probar:** primero `[profile.dev.package."*"] opt-level = 3`, en un worktree descartable que se borra junto con su directorio de build. El selector `"*"` excluye los crates del workspace, que así siguen depurables y recompilan rápido. Release usa ese mismo nivel, así que R marca el techo de lo que puede ganar.
- **Costo:** compilar desde cero tarda más (`cargo clean --workspace` conserva las dependencias, así que se compilan una vez), `target-test` crece y CI recompila hasta que `cache.yml` reescriba su caché, porque cambia un manifiesto. Las dependencias se depuran sin variables. El perfil `test` hereda de `dev`, y el tamaño del binario de prueba ya tiene un tope (`Cargo.toml:68-71`). No toca ningún vector ni producción.
- **Se queda si:** una corrida como D con la variante (puede usar la memoria) baja esa suma a la mitad, el primer build limpio de `target-test` tarda ≤ 30 % más y ocupa ≤ 20 % más, y `e5_golden` no mueve ningún bit.
- **Si no se queda:** sólo se documenta en `specs/eval` que `rag-eval --docs` se corre en release, como `load-eval`.

**W.**
- **Por qué ese umbral:** `replace_doc` prepara tres sentencias por pasaje (`mod.rs:645-660`), y `delete_doc` dos por pasaje (`:703-715`). En la importación, preparar dos sentencias por fila costaba ~16 % de un banco ([verify de import-linear](../archive/2026-10-24-import-linear/verify.md), línea 34). Si `write` es menos del 10 % del total, la ganancia queda bajo ~2 %, dentro del ruido.
- **Costo:** ninguno; lo escrito no cambia.
- **Se queda si:** `phases.write` baja ≥ 10 %.

**A1.** `embed_passages` ordena por tokens, como `longest_first`, embebe y devuelve los vectores en el orden de los pasajes antes de `fill` (`mod.rs:514`). Un doc de 16 pasajes o menos no cambia: ya es una sola tanda.
- **Por qué ese umbral:** para ordenar hay que contar tokens, y eso cuesta lo mismo que `count_seconds`. Un 10 % con una estimación que se queda corta deja lo medido cerca del 15 % que pidió la importación (verify de embed-throughput, línea 39).
- **Costo:** contar tokens por doc, y los bits (decisión 4). `by_area.{docs,code}` muestra dónde se gana. No agrega memoria ni latencia: sigue habiendo un doc a la vez.
- **Se queda si:**
  - `phases.embed` baja ≥ 10 %.
  - `tokens.padded_ran` es igual a `tokens.padded_sorted_in_doc` de R.
  - `--diff` no mueve ninguna pregunta.
  - Cumple la decisión 4.

**S.**
- **Por qué ese umbral:** solapar esconde, a lo sumo, esa parte del tiempo. Con menos del 15 %, no paga sus costos.
- **Costo:**
  - Rompe «un doc en memoria a la vez» (`mod.rs:450-452`): pasan a ser dos.
  - Un fallo al escribir llega cuando el doc siguiente ya se está embebiendo.
  - Las fases dejan de sumar `index_seconds`.
- **Se queda si:** `index_seconds` baja ≥ 10 % con `--no-cache`, y `RefreshReport` sale igual.

**A2.**
- **Por qué ese umbral:** ese número es lo máximo que A2 puede sumar a A1; una cola acotada recupera menos que ordenar todo. Como A2 cuesta más, tiene que ganar más de lo que se le pidió a A1.
- **Costo:**
  - Rompe «un doc en memoria a la vez».
  - Hoy un vector inutilizable omite sólo su doc (`mod.rs:470-473`); con la cola, el fallo tiene que volver al doc del que salió.
  - La memoria crece con el tope de la cola. Se mide con `process.private_peak_bytes` en un refresco del daemon, porque `rag-eval` no registra `process.*`.
  - En la primera indexación, los primeros docs se escriben más tarde.
- **Se queda si:**
  - `phases.embed` baja ≥ 15 % contra la corrida de A1.
  - El pico sube ≤ 5 %, el criterio de la importación (verify de embed-throughput, línea 41).
  - Cumple la decisión 4.

**E.** Hoy docs comparte con las memorias un solape de 64 tokens (`crates/clawg-core/src/memory/chunker/mod.rs:32`; `project_docs/chunk.rs:8`, `:137`). E prueba un solape propio para docs, de 32 tokens. Se prueba en un worktree descartable, y no se escribe código si no gana.
- **Por qué ese umbral:** menos del 10 % es menos de lo que se le pidió a A1, y E paga en calidad de recuperación y en un reindexado completo.
- **Costo:**
  - Cada proyecto se reindexa una vez.
  - Necesita una huella de docs que cubra cómo se parte (`docs_contract`, decisión 6 de [docs-search-quality](../docs-search-quality/proposal.md)). Sin ella, `plan_refresh` conserva los pasajes viejos (`mod.rs:124`).
- **Se queda si:** ninguna pregunta sale del top 3, y `--diff` no empeora Hit@3 ni MRR@10 con p < 0,05, mirando docs y código por separado.

## Descartados

- **B, `EMBED_BATCH`.**
  - #94 no da ningún campo que lo decida («nada directo»). Haría falta barrer valores con corridas reales sin una lectura que lo justifique.
  - Una tanda más chica sólo gana en relleno, y eso lo recuperan A1 y A2 sin tocar el embedder. Una más grande no ganó nada medido: a 16, «indexing runs as fast as with one big batch», según el comentario de `embedder.rs:179-183`, que también registra los gigabytes que pedía una tanda grande.
  - La constante vale para todo el embedder: guardados, consultas, importación (`import.rs:75`) y deshacer un borrado (`project_delete.rs:655`, `:969`). Cambiarla movería los vectores de memoria y la tanda de 17 de `e5_golden`, no sólo un índice derivado.
  - Su costo es el pico de memoria del daemon, que vigilan 62 y 91, y `rag-eval` no lo mide.
  - 62 también lo excluye, así que nadie lo toma. Se reabriría como cambio propio si, con A1 hecho, una corrida da `tokens.padded_ran / tokens.real` ≥ 1,10.
- **Tokenizar una sola vez para el partidor y el embedder.**
  - No es el mismo texto. El partidor cuenta el cuerpo dentro de su presupuesto (`project_docs/chunk.rs:132`, `:137`). El embedder codifica prefijo, breadcrumb y cuerpo, cortado a 512 tokens, en su propio hilo (`embedder.rs:240-255`, `:502-506`).
  - Lo máximo que ahorra es `tokens.count_seconds`. En la importación, tokenizar y rellenar fue el 1,5 % de `embed`, y contar el 0,5 % (propuesta de embed-throughput, líneas 14 y 17).
  - Se reabriría si R da `tokens.count_seconds` ≥ 10 % de `index_seconds`.
- **Una huella completa para que persista el store de la evaluación.**
  - Es código de producción dentro de la huella de docs-search-quality.
  - #94 descartó un store que persiste, porque el resultado dependería de qué corrió antes (su decisión 1).
  - Lo que ahorraría, `index_seconds − phases.embed` de D, es lo mismo que ataca C sin tocar producción.

## Alcance

- Leer cada puerta en D y R y anotarla en `verify.md` (el campo, el umbral, si abre) antes de tocar código.
- Cada paso cuya puerta abra, con sus tests y su corrida.
  - **C:** el perfil, o la línea en `specs/eval`.
  - **W:** las sentencias fijas de `replace_doc` y `delete_doc` (`mod.rs:637-670`, `:703-715`).
  - **A1:** el orden puro en `embed_passages`, el caso de `e5_golden` y la prueba ignorada de bits y tiempo sobre pasajes de docs.
  - **S y A2:** la forma la decide el diseño.
  - **E:** el experimento; llega a producción sólo si gana.
- `specs/project-docs` («Refresco») para lo que cambie del orden o de cuántos docs hay en memoria.

### No-alcance

- Sacar `archive/` del índice (decisión 1).
- Los hilos, la espera activa y `rayon` (62); el tamaño del tokenizer (91); cambiar el modelo o el contrato de vectores (17); qué se corta y dónde, por calidad (20); la memoria de vectores de #94 y su medición.
- Lo que está en «Descartados». Los guardados, la importación y las consultas no cambian.

## Interacciones

| Cambio | Relación |
|---|---|
| #94 docs-index-cost | Depende de él. A1 cambia el sentido de `tokens.padded`, que #94 calcula en el orden del doc: el diseño de A1 lo alinea para que el aviso de #94 («si difieren») no salte en cada corrida. Con S, las fases ya no suman `index_seconds`, y la cota del 5 % de #94 vale sólo para sus corridas. |
| 20 docs-search-quality | `archive/` sigue en el índice. E necesita su `docs_contract`. C acelera sus corridas debug. |
| 62 embedder-footprint | Corre `e5_golden` en cada combinación de hilos, así que el caso nuevo entra en su tabla. Los tiempos de un paso se comparan con los hilos que tenía su corrida base. |
| 91, 74, 17 | El costo de contar tokens de A1 depende del tokenizer (91). La prueba ignorada puede pasar al banco de 74. Un bit que se mueva más de 1e-6 es asunto de 17. |

## Cómo se mide el éxito

- **Antes de escribir código:** `verify.md` anota cada puerta con el campo leído en D y R y si abre. Los umbrales son los de esta propuesta y no se cambian después de ver los números.
- **Tests que hoy fallan**, con embedders falsos y sin dejar nada en el temporal:
  - **A1:** un doc de más de 16 pasajes llega al embedder del más largo al más corto, y sus vectores vuelven en el orden de los pasajes. Uno de 16 o menos llega como hoy.
  - **S:** el doc siguiente se lee y se parte antes de que vuelva el embebido del anterior, y un fallo de escritura sigue deteniendo el refresco.
  - **A2:** la cola respeta su tope, y un vector inutilizable afecta sólo a su doc.
  - **E:** un índice de la versión anterior se reindexa y sigue contestando mientras tanto.
  - **W** no tiene un test que hoy falle, porque no cambia lo observable. Lo cubren los tests del refresco y su medida es `phases.write`.
- **Por cada paso adoptado,** su corrida contra la anterior, en la misma máquina y sobre el commit anterior más el paso, cumple «se queda si». En los pasos que no cambian ni texto ni vector (C, W, S, y A1 y A2 si los bits no se mueven), `rag-eval --real --docs --diff` no mueve ninguna pregunta.
- La suite y los dos clippy pasan.

## Datos y vuelta atrás

- **C, W y S, y A1 o A2 con los mismos bits:** no cambia nada guardado, y el índice de producción se escribe igual. Para volver atrás se revierten los commits.
- **A1 o A2 con bits movidos ≤ 1e-6** (decisión 4):
  - Un doc que se embebe de nuevo trae vectores que difieren en los últimos bits. Un doc sin cambios conserva los suyos (`plan_refresh`, `mod.rs:124`), y uno cambiado reutiliza por texto, así que nunca se mezclan vectores de un mismo texto.
  - No cambian las memorias ni el esquema de `memory.db`, y tampoco `model_id`.
  - Para volver atrás se revierte el commit, con su fixture, y se vacía otra vez `docs/eval/cache/`.
- **E:** al cambiar los pasajes, cambia la huella de docs y cada proyecto se reindexa una vez (un primer indexado completo). Las memorias no se tocan. Una versión anterior lee otra huella y reindexa sin perder nada, porque el índice se deriva del disco. El test con un índice de la versión anterior está en «Tests que hoy fallan».

## Riesgos

| Riesgo | Mitigación |
|---|---|
| El corpus crece con cada cambio archivado, y una comparación mezcla commits | Cada comparación usa corridas del mismo commit y la misma `index.machine` |
| Ordenar mueve bits en otra CPU | Decisión 4: `e5_golden` y la prueba de bits en cada CPU donde se mide |
| Mover un umbral después de ver el número | Los umbrales están fijados acá, y `verify.md` los anota antes del código |
| Cada corrida necesita el modelo real, el perfil dev y autorización | Ningún paso se cierra sin su corrida |
