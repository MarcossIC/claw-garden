# embed-throughput — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M (la medición y el orden por largo son S; INT8 es L y queda condicionado) | piece-boundaries (fija cuántos tokens lleva cada fila antes de medir); se coordina con embedder-footprint, perf-bench, model-reembed e import-job | exploración (borrador) |

**Tesis:** la fase de embeddings de una importación grande (≈1.280 s para 52.080 filas, ≈40 filas/s, en un binario debug) es el tramo más largo que queda cuando la pasada de decisión y la escritura se arreglen, y nadie sabe de qué está hecha. La estimación de FLOP dice que la inferencia domina, pero esa cuenta no ve ni el padding (cada tanda se rellena hasta su texto más largo, y una fila de un solo pedazo llega a 480 tokens junto a piezas de ≈150), ni el costo de tokenizar y de agrupar dentro del hilo del embedder, ni que ese hilo corre en un build debug cuyas dependencias de Rust (tokenizers, SQLite) no están optimizadas aunque ONNX Runtime venga precompilado. El cambio es de **medición primero**: tres números (la razón entre tokens rellenos y reales, el reparto del tiempo entre tokenizar, inferir y agrupar, y lo mismo con las dependencias optimizadas) deciden cuál de cuatro brazos vale. El único brazo barato y seguro a priori es ordenar las filas por largo en el llamador (la importación ya guarda los vectores por id, así que el orden no importa); la cuantización INT8 cambia el contrato de embedding y obliga a re-embeber toda tienda existente, y por eso sólo entra si la medición muestra una ganancia grande y `rag-eval` no empeora. El resultado correcto puede ser cerrar con el ordenamiento y el registro de lo medido.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| La importación embebe sólo las filas insertadas | **Cierta.** `to_embed` filtra `deleted_at.is_none()` y `placed[..].inserted()` (`crates/clawg-core/src/memory/portability/import.rs:984-988`); lo que ya estaba presente o se fusionó no se vuelve a embeber. |
| El bucle de embeddings va de a 32 y el embedder lo parte en 16 | **Cierta.** `EMBED_BATCH` = 32 (`import.rs:57`, usado en `:990`); `E5Embedder::embed_batch` parte en `texts.chunks(EMBED_BATCH)` del embedder, 16 (`embedder.rs:173`, `:418-420`), manda una tanda al hilo y espera la respuesta antes de la siguiente (`:425-430`). |
| «Tandas más chicas en la importación acortan la espera de una consulta detrás de la tanda en vuelo» | **Falsa tal como está.** El 32 de la importación sólo secuencia dos tandas del embedder; la tanda en vuelo es siempre una de 16 (`embedder.rs:420`). Un solo productor que espera la respuesta de una tanda antes de mandar la siguiente deja a una consulta esperar, como mucho, la tanda en curso. Lo único que acortaría esa espera es bajar la tanda del embedder, y eso queda fuera (ver «Descartado»). |
| Se tokeniza dos veces (el chunker y el embedder) y se podrían reusar los ids | **Cierta la doble tokenización, falsa la reutilización directa.** El embedder tokeniza dentro de su hilo, en cada tanda (`Inference::embed` → `window_encodings`, `embedder.rs:483-486`, `:229-243`), sobre el texto con el prefijo `"query: "` (`apply_role_prefix`, `:185-196`, `:421-423`) y, para una pieza, sobre `contexto + "\n…\n" + pieza` (`embedding_text`, `chunker/mod.rs:66-72`; `import.rs:991`). El chunker cortó la pieza y la cabecera por separado (`chunker/chat.rs:114-122`). Los ids de unos no son los de la concatenación: el propio chunker lo dice, «token counts are not additive across the seam» (`chunker/mod.rs:42-44`). Reusar ids cambiaría los vectores. |
| El padding puede ser grande | **Plausible, no medido.** `pad_right` rellena cada texto hasta el más largo de la tanda (`embedder.rs:246-268`). Una fila de un solo pedazo mide hasta `CHUNK_MAX_TOKENS` = 480 (`chunker/mod.rs:26`, `chat.rs:117-118`); una pieza de un guardado largo mide a lo sumo cabecera (48, `:40`) + separador + pieza (96, `:50`) + prefijo y tokens especiales. La importación entrega las filas en el orden del archivo (`prepared.rows`, `import.rs:984-989`), así que las piezas de una nota quedan juntas (largo parecido) y las notas cortas de un solo pedazo quedan mezcladas entre ellas. Cuánto cuesta esa mezcla es justo el número que falta. |
| La inferencia domina el tiempo | **Estimación consistente, no medida.** ≈21 M parámetros fuera de los embeddings (12 capas × (4·384² + 2·384·1536)) son ≈42-45 MFLOP por token; 52.000 filas × ≈150 tokens ≈ 7-8 M tokens ≈ 330 TFLOP; en 1.280 s son ≈250 GFLOP/s. Es la cota inferior de cálculo útil: el padding sube el real. |
| El pooling y la normalización pesan | **Se descarta por aritmética, y se mide igual.** `pool_and_normalize` (`embedder.rs:290-335`) gasta unas 384 multiplicaciones-sumas por token, contra ≈21 M de la inferencia (cinco órdenes de magnitud); aun con un factor 30 por ser debug, el pooling queda bajo el 1 %. Recorre las posiciones de padding (`:313`, suma `0 * x`), pero eso es lineal en el relleno y despreciable. |
| El perfil debug explica parte de los 40 filas/s | **Cierta por construcción, a cuantificar.** ONNX Runtime viene precompilado (`ort` con `download-binaries`, `Cargo.toml:43`), así que la inferencia no depende del perfil; `tokenizers` (`:52`) y SQLite incluido se compilan con el perfil activo, y el `Cargo.toml` raíz sólo define `[profile.test]` (`:71-72`), sin `[profile.dev.package]`. La tokenización, el armado de tensores y el pooling sí corren sin optimizar en la medición de 1.280 s. |
| Tanda 16 y arena apagada se mantienen | **Cierta, con la evidencia del brief.** `cpu_provider()` apaga la arena (`embedder.rs:154-160`) y la tanda es 16 (`:168-173`). Medido el 2026-10-15 con `rag-eval --real --docs` sobre este repositorio (265 documentos, 5.924 pasajes, pico de bytes privados): tanda 256 con arena, 17.321 MB y 751 s; tanda 16 con arena, 2.171 MB y 741 s; tanda 16 sin arena, 1.602 MB y 810 s, con los mismos resultados. No se reabre. |
| Los hilos de ONNX son los lógicos | **Cierta.** `build_session` pide `available_parallelism()` hilos intra-op (`embedder.rs:513-526`, hilos en `:515`, `:522`). Las citas de embedder-footprint (`:500-513`) derivaron: hoy la función está en `:513-526`. Es el terreno de embedder-footprint, no de este cambio. |
| El re-embeber agrupa en tandas | **Falsa.** Infiere fila por fila: `embed_with_role(&row.text, EmbedRole::Passage)` (`maintenance.rs:689`), 200 filas leídas por vuelta (`:427`). Cualquier mejora de tanda no lo toca sin model-reembed (17). |

## Problema (verificado contra el código)

### 1. Qué se sabe y qué no de los 1.280 s

La medición real separó cuatro fases de la importación; la de embeddings fue ≈1.280 s con la conexión del store libre (las búsquedas contestaron en 0,02-1,1 s). Nada separa, dentro de esa fase, tokenizar, inferir y agrupar. El camino de una tanda es: la importación arma 32 textos (`import.rs:991`), el embedder los parte en 16, un `mpsc` sin tope los lleva al hilo `embedder-worker` (`embedder.rs:543-563`), que en una sola pasada tokeniza con paralelismo (`encode_batch`, `:236`), rellena, corre la sesión, hace pooling y responde (`:483-505`); el productor recién entonces manda la siguiente (`:425-430`). No hay solapamiento entre tokenizar la tanda siguiente e inferir la actual.

### 2. Dónde puede haber desperdicio

- **Padding.** Cada texto de una tanda se rellena al más largo y la atención crece con filas × tokens² (comentario `embedder.rs:168-172`). Con un orden de archivo, una tanda de 16 notas cortas de un solo pedazo que incluye una de 480 tokens paga 480 en las 16. Ordenar por largo lo reduce sin tocar el contrato.
- **Tokenización y armado dentro del hilo de inferencia.** Corre en debug y en serie con la sesión. Si pesa un 5-10 % de la fase, el solapamiento la esconde; si pesa 1 %, no vale.
- **Cómputo en FP32.** Sólo INT8 lo cambia, y cambia los vectores.
- **Hilos.** `available_parallelism()` hilos intra-op con la espera activa por defecto; lo que toque eso pertenece a embedder-footprint, que mide CPU, tiempo y bits antes de moverlos.

### 3. Qué se hace con un resultado

El orden por largo no necesita nada nuevo en el contrato ni en el esquema: la importación guarda los vectores en un `HashMap` por id de fila (`import.rs:989`, `:995`) y la escritura los busca por id (`:1009-1018`), de modo que mandar las filas al embedder en otro orden no cambia qué vector recibe cada una. Lo que sí puede cambiar son los bits: otra composición de tanda es otra forma de tensor (`PaddedBatch { rows, len }`, `embedder.rs:213-219`), y `pool_and_normalize` sólo garantiza el orden de las operaciones dentro de una fila (`:283-289`), no que otra forma de tanda dé los mismos bits (el lote de 17 de `e5_golden` lo ejerce, `tests/e5_golden.rs:211`).

## Lo que ya existe y se reutiliza

- `EvalTokenCounter::real` (`crates/clawg-core/src/eval_api.rs:845-859`) y `tokenizer.json` bajo `<data>/models/multilingual-e5-small` (pinneado en `model_download.rs:64`, `:98-100`): permiten contar tokens reales de cada fila con el tokenizer solo, sin sesión ni modelo ONNX.
- `window_encodings` y `pad_right` (`embedder.rs:229`, `:248`) son funciones puras: la razón tokens rellenos / tokens reales de un orden dado se calcula sobre sus resultados sin inferir.
- `from_mock_worker` (`embedder.rs:653-665`, `#[cfg(test)]`) y `serve_requests` (`:579`) para probar la composición de tandas sin modelo.
- El embedder falso del import (`FakeEmbedder`, usado en `import.rs:1286`) y `CLAWG_TEST_EMBEDDER=fake` para el banco sin modelo.
- `e5_golden` (`crates/clawg-core/tests/e5_golden.rs`): compara bits sólo con la misma huella de CPU y tolera 1e-6 en otra (`:26`, `:47-53`); `CLAWG_BLESS=1` reescribe la fixture (`:13-14`).
- `rag-eval --real`, `--diff` y `--fail-on-regression` (`.claude/CLAUDE.md`, «Testing») para cualquier brazo que mueva vectores.
- `crates/clawg-cli/tests/transport_bar.rs` como patrón de banco ignorado, y perf-bench (74) como lugar donde ese banco se generaliza.
- Las métricas `embedder.batch`, `embedder.batch_tokens` y `embedder.wait` que pipeline-metrics (73) propone: son la forma de ver el padding en uso real.

## La puerta: qué se mide antes de elegir

Se corre sobre el mismo export de engram de la medición de referencia (o sobre un set sintético con la misma distribución de largos si el real no se puede compartir; el resultado dice cuál) y sobre las filas que el daemon decidiría insertar. Cada medición dice el perfil (debug o release), el sistema y la CPU.

| Medición | Cómo | Qué decide |
|---|---|---|
| **M1. Razón de relleno** = tokens enviados al modelo (filas × largo de tanda) / tokens reales | Sólo con el tokenizer: `window_encodings` + `pad_right` sobre las filas a insertar, en el orden del archivo y ordenadas por tokens; tandas de 16 | Si el orden de archivo ya está cerca de 1,0x, el brazo A se cierra. Si pasa de ≈1,3x, A es la primera acción |
| **M2. Reparto del tiempo por tanda** | Reloj por etapa en el hilo del embedder para 2.000 filas: tokenizar, armar tensores, `session.run`, pooling; en debug y con las dependencias optimizadas (un override de `opt-level` para dependencias en un perfil de medición, o un banco en `target-test` como `transport_bar.rs`; un build de release exige autorización explícita) | Decide B (solapar la tokenización: sólo si tokenizar+armar pasa de ≈10 %), y si los 40 filas/s son un artefacto de debug |
| **M3. Filas/s con modelo real** | La fase de embeddings sola (sin SQLite) sobre 5.000 filas, en el orden del archivo y en orden por largo; mediana de 3 corridas | Mide la ganancia real de A (no sólo la razón de M1) |
| **M4. Bits** | `e5_golden` y una prueba de pares: cada texto embebido solo y dentro de una tanda ordenada, diferencia máxima | Si el orden mueve bits, decisión abierta (tolerancia o fixture); por defecto no entra sin decisión |
| **M5. Memoria** | Pico de bytes privados con A (tandas de 16 de largo homogéneo; el pico depende de la tanda más larga, que A no cambia) | Debe ser ≤ la medición de 1.602 MB; si no, A se ajusta |
| **M6. INT8** (sólo si M3 muestra que la inferencia domina) | Tiempo por tanda y `rag-eval --real` con una variante cuantizada en un perfil dev | Ver brazo C |

## Enfoques posibles

### A. Ordenar las filas por largo en el llamador (recomendado si M1 lo justifica)

La importación ordena `to_embed` por tokens (o por bytes del texto como proxy, sin tokenizar) antes del bucle de `import.rs:990`, manda tandas de 16 de largo parecido y guarda cada vector por id, como ya hace. **A favor:** no toca el embedder ni el contrato, no cambia cuántos vectores hay ni cuál va a cuál fila, es una función pura probable sin modelo (índices de entrada a tandas, con la salida reordenada), y es el único brazo cuyo costo es casi nulo. **En contra:** mueve la composición de tanda, así que puede mover bits (M4) y el mismo texto puede quedar con un vector distinto en el último decimal según con quién comparta tanda; el orden por bytes es un proxy (idiomas con muchos tokens por byte lo desordenan algo; con el tokenizer el costo es una pasada extra, que la importación puede amortizar si 83 deja el recuento a mano). Alternativa dentro del embedder: ordenar en `embed_batch`, pero la importación entrega sólo 32 por llamada (`import.rs:990`), una ventana demasiado chica para que el orden rinda; por eso el lugar es el llamador. Otros llamadores de tandas (el refresco de docs, `project_docs/mod.rs`) pueden adoptar la misma función pura después.

**Variante A2: presupuesto de tokens por tanda.** Cortar la tanda por un tope de filas × largo en lugar de fijo en 16, de modo que las filas cortas lleven más filas y las largas menos. Reabre la tanda de 16 y el pico de memoria; sólo con una medición de M3/M5 que lo sostenga.

### B. Solapar la tokenización de la tanda siguiente con la inferencia de la actual

Hoy una tanda pasa por tokenizar, armar e inferir en el mismo hilo (`embedder.rs:483-505`). Se puede tokenizar fuera del hilo del modelo y mandar `PaddedBatch` ya armado, o encadenar dos etapas. **A favor:** si tokenizar pesa ≥10 % (M2), lo esconde. **En contra:** cambia la forma de `EmbedRequest` (`:405-408`), hoy sólo textos, y el reparto de CPU con la inferencia (ONNX ya usa todos los hilos). No cambia bits. Sólo si M2 lo pide.

### C. INT8 (cuantización dinámica del modelo), condicionado a M6 y a model-reembed (17)

Un modelo cuantizado es otro modelo para el contrato: otros pesos, otros vectores. Para entrar hay que cambiar **todo** esto:
- `E5_MODEL_ID` (`embedder.rs:135`) y `expected_contract()` (`:145-152`) a un id nuevo, que todo store existente ve como `Mismatch` (`db.rs` compara byte a byte; `bootstrap.rs:519` `is_pooling_upgrade` repara sólo el cambio de pooling CLS → promedio, y `verify_or_reembed` está en `bootstrap.rs:533`). Cualquier otro cambio apaga la memoria hasta una reparación que no existe (model-reembed, «Qué repara hoy el arranque» y «todo o nada», `openspec/changes/model-reembed/exploration.md:38-58`).
- El manifiesto de descarga: hoy un solo `onnx/model.onnx` con SHA-256 fijo (`model_download.rs:62-63`, `:93-96`, commit `614241f6…`, `:37-40`). Un archivo cuantizado necesita su origen verificado, su hash y su tamaño; cada archivo se verifica contra su hash aunque venga de un espejo (`.claude/CLAUDE.md`).
- Una prueba que siembre un store con el contrato anterior y demuestre que se re-embebe o se rechaza sin tocarlo (regla de `openspec/config.yaml` para el contrato).
- Calidad con `rag-eval --real` y `--diff`, con `recall@5` y `search.found` como métricas primarias, y el test `e5_golden`.
- **CPU.** Los kernels INT8 de ONNX Runtime van más rápido con VNNI/AVX-512; en una CPU sin esas extensiones un modelo cuantizado puede ser **más lento** que FP32. El binario de release apunta a cualquier x86_64 (`release.yml`), de modo que haría falta detectar la CPU o fijar un valor por defecto medido. Dos modelos según la CPU significan dos espacios de vectores en la misma instalación: un contrato que depende del hardware es incompatible con mover el store entre máquinas (`clawg export`/`import`, `restore`). Lo más probable es una sola opción para todos o ninguna.
- **Orden respecto de la importación de producción.** Si INT8 se decide después de importar (orden acordado), cada fila importada con FP32 se re-embebe: otra pasada de la misma magnitud que la fase que se quiere acelerar. Eso no invalida el orden, pero fija la comparación: la ganancia de INT8 debe pagar ese re-embeber, y la comparación se hace con el número de M3.

### D. Hilos, espera activa y paralelismo del tokenizer

Es el terreno de embedder-footprint (62): `available_parallelism()` en `embedder.rs:515`, `with_intra_op_spinning` por defecto activado, el grupo de `rayon` del tokenizer. Este cambio no los mueve. Pero el throughput es una columna que esa tabla de puerta necesita (más hilos suben filas/s y bajan el CPU libre del usuario): ver «Interacciones».

### E. Sin código: dejar el registro de lo medido

Si M1 da ≈1,0x, M2 da que la inferencia es ≥90 % y M3 confirma que el release a velocidad de modelo es aceptable (fase de embeddings en el orden de minutos con un binario release), se cierra con la medición registrada.

### Recomendación: M1 a M5, luego A; B sólo si M2 lo pide; C sólo si M3 y M6 lo piden

1. Medir M1 (sólo tokenizer, minutos) y M2 en debug y con dependencias optimizadas. Esas dos mediciones deciden qué es un artefacto de debug.
2. A, con su función pura y sus RED, si M1 > ≈1,3x o si M3 muestra una ganancia material.
3. B y C se evalúan con sus propias puertas; C no entra sin la reparación general de contrato que model-reembed describe.
4. El override de optimización de dependencias en el perfil de desarrollo (un `[profile.dev.package."*"]`) es una decisión de herramientas del repositorio (tiempo de compilación contra velocidad de las pruebas con modelo real); se decide con M2, no antes.

## Descartado

| Opción | Razón técnica |
|---|---|
| Cola de prioridad en el embedder para bajar la latencia de búsqueda durante la importación | Un único productor ya espera la respuesta de cada tanda antes de mandar la siguiente (`embedder.rs:425-430`), así que una consulta espera, como mucho, la tanda en vuelo; una prioridad no acorta la tanda que corre. |
| Prefijo `passage: ` | Medido peor (`embedder.rs:122-129`); además cambia el contrato (`:111-130`). |
| Piezas de 350-450 tokens | Medido en long-save-ranking: la respuesta visible pasó de 0,000 a 0,917 con piezas de 96/24 frente a 424 en los 12 casos largos. |
| Reactivar el arena de ONNX Runtime | Mismos resultados con 1.602 MB contra 2.171 MB y 17.321 MB (ver premisas); `cpu_provider()` la apaga a propósito (`embedder.rs:154-160`). |
| Subir la tanda por encima de 16 | La atención crece con filas × tokens² y la tanda 256 tomó 17.321 MB para el mismo tiempo (751 s contra 741-810 s). |
| Reusar los ids del chunker | No son los de la concatenación con el prefijo y la cabecera (`chunker/mod.rs:42-44`, `embedder.rs:185-196`); cambiaría vectores. |
| Bajar la tanda del embedder por debajo de 16 para acortar la espera de una consulta | No es un objetivo de este cambio (la espera es una tanda); se mediría con `embedder.wait` de 73 si alguien lo pide. |
| Saltar las posiciones de padding en el pooling | Despreciable (menos del 1 % aun en debug); además cambia el comportamiento ante valores no finitos (`0 * inf` es NaN hoy, `to_unit` lo rechaza después). |
| Embeddings diferidos (filas sin vector) | Una fila sin vector no la encuentra ningún canal de búsqueda (`search.rs:1052-1068` puntúa los candidatos léxicos por distancia vectorial); se descartó para esta hoja de ruta. |

## Interacciones

- **embedder-footprint (62).** Su puerta mide CPU, tiempo y bits antes de tocar hilos o tandas, en Linux y Windows, con un modelo real. Se recomienda **que 85 no la absorba**: los dos cambios tocan el mismo hilo del embedder pero responden preguntas opuestas (62: cuánto CPU deja libre al usuario y cuánta memoria guarda; 85: cuántas filas por segundo y cuánto relleno), y ambos miden con el mismo banco. Lo que sí se comparte es la tabla de mediciones: 85 aporta el escenario «importación de 50.000 filas» (filas/s, razón de relleno, reparto por etapa) y 62 aporta las columnas de CPU y de memoria; la decisión sobre hilos y espera activa se toma **una vez**, en 62, con las dos columnas a la vista. Si 62 se cierra antes con un cambio de hilos, 85 vuelve a medir M3 después. Ambos cambian cerca de `build_session` (`embedder.rs:513`): coordinar el orden de commits.
- **perf-bench (74).** El banco ignorado con embedder falso aísla SQLite y ranking; M2 y M3 necesitan el modelo real, que es el modo `--real` de 74. Si 74 llega antes, M3 es un escenario suyo; si no, M3 se escribe como prueba ignorada en `clawg-core/tests` o `clawg-cli/tests` y 74 la adopta. Un build de release es de `transport_bar.rs`: exige autorización explícita.
- **pipeline-metrics (73).** `embedder.batch_tokens` y `embedder.wait` dan la razón de relleno en uso real, no sólo en un banco; A debería poder verse ahí.
- **model-reembed (17).** Cualquier brazo que cambie el modelo (C) depende de la reparación general del contrato que 17 describe; el re-embeber de arranque infiere fila por fila (`maintenance.rs:689`), de modo que si 17 agrupa en tandas, A (en forma de función pura reutilizable) le sirve.
- **piece-boundaries (83).** Cambia cuántos tokens lleva cada fila y cómo empieza (hoy las piezas empiezan a mitad de palabra); como la importación no vuelve a cortar una nota presente, 83 va antes. Por eso M1 y M3 se corren **después** de 83, sobre las filas que de verdad se van a importar.
- **import-job (87).** La importación pasa a correr como un trabajo con avance; la fase de embeddings es su mayor tramo y su estimación de tiempo restante depende de filas/s. El orden por largo hace que ese ritmo no sea constante (las tandas largas quedan al final o al principio según el orden): decidir si el avance se mide en filas o en tokens.
- **import-linear (81) e import-decision-pass (82).** Las dos acortan las fases de decisión y de escritura; con ellas hechas, la fase de embeddings pasa a dominar el total y su peso relativo sube.
- **Specs:** `openspec/specs/storage/spec.md` («Ciclo de vida del embedder») sólo si cambia algo observable; el contrato de embedding sólo si C entra.

## Alcance candidato

1. La medición (M1-M5) y su registro por escrito, con el perfil, el sistema y la CPU.
2. Una función pura que, dados los largos, devuelve las tandas (índices) para que un llamador embeba por largo, y su uso en la importación (A).
3. Si M2 lo pide, el solapamiento de tokenización (B).
4. Si y sólo si M3, M6 y la reparación de contrato lo sostienen, C como cambio aparte con su propio ciclo (esquema de contrato, prueba de migración, `rag-eval --diff`).

### No-alcance

- Cambiar la tanda del embedder, el arena o los hilos (62).
- Cambiar el modelo, el contrato, los prefijos o el tamaño de pieza sin una decisión explícita.
- Dejar filas sin vector o diferir sus embeddings.
- Una cola de prioridad en el embedder.
- Agrupar el re-embeber (17).
- Dejar fuera filas de la importación (los resúmenes de sesión no se omiten; su peso en el ranking es de otro cambio).

## Decisiones abiertas

Son del usuario.

- [ ] **Umbrales.** ¿Qué razón de relleno (M1) justifica A, y qué ganancia de filas/s (M3)? Candidato: relleno > 1,3x o ganancia > 15 % de la fase.
- [ ] **Perfil de medición.** ¿Se autoriza un build de release en `target-test` para M2/M3, o se usa un override de optimización de las dependencias en un perfil de medición? El primero da el número del producto; el segundo no cambia lo que se publica.
- [ ] **Orden por tokens o por bytes.** Por tokens es exacto y cuesta una pasada del tokenizer; por bytes es gratis y menos preciso. Candidato: bytes si M1 muestra que el proxy alcanza.
- [ ] **Bits.** Si el orden mueve bits (M4): ¿se acepta la tolerancia de 1e-6 de `e5_golden`, o se reescribe la fixture, o A no entra? Por defecto no entra nada que mueva bits sin decisión.
- [ ] **INT8.** ¿Se explora al final de M3 con una variante cuantizada y `rag-eval --real`, o se descarta de entrada por el costo de contrato y de re-embeber? Si se explora: ¿cuál es el origen verificado del archivo cuantizado y quién lo mantiene?
- [ ] **¿Absorbe 85 la puerta de 62?** Recomendado: no; una tabla de mediciones compartida con una sola decisión sobre hilos (ver «Interacciones»).
- [ ] **Avance de 87.** ¿En filas o en tokens?

## Cómo se mide

**La medición (M1-M5) es el primer entregable**, sin código de producción: una prueba ignorada (necesita el tokenizer y, para M2-M5, el modelo, vía `CLAWG_E5_MODEL_DIR` como `e5_golden`) que imprime la razón de relleno por orden, el reparto por etapa y filas/s, con el perfil y la CPU.

Tests RED, cada uno falla hoy:

1. **El orden de la importación agrupa por largo.** Con un embedder de prueba que registre las tandas que recibe y filas de largos mezclados (notas cortas de un solo pedazo entre piezas), la razón entre el largo máximo y el mínimo dentro de cada tanda de 16 es ≤ un tope. Hoy las tandas siguen el orden del archivo y la razón es la de la mezcla. El mismo informe de importación byte a byte con el orden por largo.
2. **Cada fila recibe el vector de su texto.** Con el embedder falso, determinista por texto, el vector guardado de cada fila es el de `embedding_text(contexto, contenido)` sin importar el orden. Pasa hoy: es la red de seguridad de A.
3. **La función de tandas es total y estable.** Cada índice aparece una vez, el orden de salida a los llamadores se restituye, el empate se rompe por posición (sin azar). Hoy no existe.
4. **Bits.** `e5_golden` y una prueba ignorada con el modelo real: la diferencia máxima entre un texto embebido solo y dentro de una tanda ordenada, bajo la tolerancia. Hoy sólo existe el lote de 17.
5. **(Si C)** Un test que siembra un store en el contrato anterior (`multilingual-e5-small/mean`) y abre con el id nuevo: el resultado es un `Mismatch` sin tocar filas, vectores ni contrato, o la reparación que 17 defina; hoy no hay id nuevo que probar. Y `rag-eval --real --diff` con `recall@5` y `search.found`, sin empeorar con p < 0,05 (`--fail-on-regression`).
6. **(Si B)** Una prueba con `from_mock_worker` que muestre que la tokenización de la tanda siguiente ocurre antes de que termine la inferencia de la actual. Hoy cada tanda espera a la anterior de punta a punta.

El éxito del cambio es, para A, M3 (filas/s) por encima del umbral en el orden por largo con los mismos resultados de importación; para el cierre sin código, la medición registrada.

## Riesgos

- **Que la medición muestre que no hay nada que ganar.** Es un resultado válido (enfoque E).
- **Que los 40 filas/s sean un artefacto de debug.** Entonces el problema es de la herramienta de medición, no del producto; se registra y la importación de producción se estima con el release.
- **A mueve bits.** Mitigación: M4 y la decisión abierta; el vector es una función de la composición de tanda.
- **Orden por bytes que no refleja tokens.** Mitigación: M1 lo cuantifica con el tokenizer; si no alcanza, por tokens.
- **Orden global y memoria.** Ordenar 52.000 textos antes de embeber exige tener todos en memoria (ya lo están: `to_embed` es un `Vec<&Row>`, `import.rs:984-988`), pero el pico del embedder lo fija la tanda más larga, que A no cambia.
- **INT8 más lento sin VNNI.** Mitigación: sólo con una medición en una CPU sin esas extensiones, o una sola opción medida.
- **INT8 después de la importación obliga a re-embeber 52.000 filas.** Es parte de su costo y se cuenta en la comparación.
- **Los números de debug no son los de release.** Cada medición dice qué perfil usó.
