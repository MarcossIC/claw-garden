# embed-throughput — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S | 83 (archivado); se coordina con 62, 74, 87 y 17 | ver [`state.yaml`](state.yaml) |

## Problema

La fase de embeddings es casi toda la importación. En prod, con el release 0.62.0, tomó 1.115,0 de los 1.146,9 s del import del historial (97,2 %, 54.436 filas, 48,8 filas/s). En el perfil dev, con un build debug, 1.289,1 s (91,3 %, 42,1 filas/s; [verify de import-timing](../2026-10-24-import-timing/verify.md#la-importación-real-en-el-perfil-dev-windows-2026-10-24)). El release embebe sólo un 13,5 % más rápido que el debug, así que la fase es inferencia de ONNX Runtime, que llega precompilado en los dos.

La [exploración](exploration.md) dejó la medición como primer paso. Se midió con `the_embedding_phase_of_a_real_export` (un test ignorado, ver [spec](spec.md#et3-la-medición-de-la-fase-de-embeddings)) sobre un `engram export --all` con las mismas 3.800 observaciones y 14 prompts del import en prod. Al store vacío le da 54.321 filas para embeber, con 7.025.016 tokens (129,3 por fila). Build debug, Windows x86_64, 16 CPUs lógicas, con AVX2, AVX-512F y FMA.

- **M1, el relleno.** La importación toma las filas en el orden del archivo (`crates/clawg-core/src/memory/portability/import.rs:1629-1633`) y llama al embedder de a 32 (`EMBED_BATCH`, `import.rs:66`, `:1636`). El embedder corta en corridas de 16 (`crates/clawg-core/src/memory/embedder.rs:173`, `:420`) y rellena cada texto de una corrida hasta el más largo (`pad_right`, `embedder.rs:248-268`). Hay 137 filas de hasta 64 tokens, 23.260 de 65 a 128, 30.081 de 129 a 256 y 843 de 257 a 512: las piezas de una nota larga, de ~130 a 150 tokens, quedan mezcladas con notas de un solo pedazo de hasta 480. En el orden del archivo, el modelo corre sobre **1,293 veces los tokens reales y 2,042 veces la atención** (que crece con el cuadrado del largo de una corrida). Por bytes del texto quedan 1,064 y 1,131; por caracteres, 1,063 y 1,129; por tokens, 1,001 y 1,002.
- **M2, las etapas.** De lo que tarda el orden del archivo, tokenizar y rellenar es el 1,5 % y el pooling el 1,9 %, en debug. El resto, ~96,6 %, es la sesión.
- **M3, el tiempo.** Sobre las primeras 5.000 filas, mediana de 3 corridas alternadas: orden del archivo 107,1 s (46,7 filas/s); por bytes 72,5 s (−32,3 %); por caracteres 72,4 s (−32,3 %); por tokens **67,5 s (74,1 filas/s, −37,0 %)**. La muestra rellena más que el total (1,458x contra 1,293x de los tokens), así que la ganancia del total es menor y se mide aparte (ver «Cómo se mide»).
- **M4, los bits.** Las 5.000 filas dan el mismo vector, bit por bit, en los cuatro órdenes, y 65 de ellas, repartidas por largo, dan el mismo que embebidas solas.
- **Contar los tokens** de las 54.321 filas tarda 5,4 s en debug, el 0,5 % de la fase.

## Objetivo

Transversal: la importación (objetivo 1, guardar de forma eficiente) embebe los mismos vectores en menos tiempo, y la forma de ordenar queda para el próximo que embeba por tandas (el re-embebido de 17). Lo que se escribe no cambia.

## Decisiones

Cierran las decisiones abiertas de la [exploración](exploration.md#decisiones-abiertas). El detalle y las alternativas están en el [design](design.md).

1. **Umbrales.** Los de la exploración, fijados antes de medir: el brazo A entra con un relleno mayor que ~1,3x o con más del 15 % de la fase. M1 da 1,293x de tokens, en el borde, pero 2,042x de atención; M3 da −37,0 % en la muestra. La importación completa tiene que confirmarlo con el mismo 15 %.
2. **Perfil de medición: debug en `target-test`, sin build de release.** La sesión es la misma en los dos perfiles, porque ONNX Runtime llega precompilado (`ort` con `download-binaries`). Lo que gana un orden lo gana la sesión. El reparto release contra debug ya lo dio el import en prod.
3. **Orden por tokens, no por bytes.** Saca 5 s más por cada 5.000 filas que el orden por bytes (67,5 contra 72,5 s), y contarlos cuesta 5,4 s para todas las filas. Se cuentan con el tokenizer del daemon, el mismo que usa el embedder, a través del `ChatChunker` que la importación de otra herramienta ya recibe. Sin tokenizer, la cuenta del chunker cae a un cuarto de los bytes, y el orden queda por bytes.
4. **La más larga primero, y los empates en el orden del archivo.** La corrida que fija el pico de memoria (16 textos de 512 tokens) va al principio. Así, una importación que la máquina no aguanta falla en sus primeros segundos y no después de un cuarto de hora.
5. **Bits.** No mueve ninguno en la máquina medida. No hace falta tolerancia ni fixture nueva. `e5_golden` no cambia, porque embebe por `EvalEmbedder` y no por la importación. En otra CPU, el vector de una fila ya depende hoy de con quién comparte la corrida (el orden del archivo es tan arbitrario como cualquier otro), así que el orden no agrega una clase de diferencia nueva.
6. **B, solapar la tokenización, se descarta.** Pedía que tokenizar y armar pasaran del ~10 %, y son el 1,5 %.
7. **C, INT8, queda fuera.** Necesita la reparación general del contrato de embedding que 17 describe y que no existe (`crates/clawg-core/src/memory/bootstrap.rs:520` sólo repara el cambio de pooling CLS → promedio). Re-embebería las ~55.600 filas de prod, pide un origen verificado para el archivo cuantizado y dependería de la CPU (VNNI). Si alguna vez vale, es un cambio propio después de 17.
8. **62 no se absorbe.** Las columnas de este cambio (filas/s, relleno y etapas) van a la tabla de mediciones de embedder-footprint. Hilos y spinning siguen siendo de 62.
9. **El avance de 87.** Con la más larga primero, las filas/s suben a lo largo de la fase, y un avance contado en filas subestima al principio. Lo decide 87. Este cambio ya cuenta los tokens de cada fila antes de embeberla.
10. **Optimizar las dependencias en el perfil de desarrollo** (`[profile.dev.package."*"]`) no se adopta: tokenizar es el 1,5 % de la fase.

## Alcance

1. La medición: `the_embedding_phase_of_a_real_export` (ignorado) y `padding`, la cuenta pura del relleno, con su test.
2. El orden: `longest_first` (pura), `embedding_order` (cuenta los tokens de las filas en grupos, en un hilo bloqueante, y las ordena), `ChatChunker::count_all` y su `TokenSplitter::count_all`. `write` embebe en ese orden. `import` recibe el chunker, como ya lo recibe `import_foreign`, y `import_memories` (`crates/clawg-cli/src/daemon/api.rs:1943-1947`) se lo pasa a las dos.
3. `.claude/CLAUDE.md` (cómo embebe una importación), el delta de `specs/portability` y la fila 85 de `openspec/README.md`.

### No-alcance

Lo que deja fuera la [exploración](exploration.md#no-alcance): la tanda del embedder, el arena, los hilos (62), el modelo, el contrato, los prefijos, el tamaño de pieza, filas sin vector, una cola de prioridad y agrupar el re-embebido (17). Además:

- El refresco del índice de docs (`project_docs`) sigue con su orden. Puede adoptar `longest_first` en un cambio propio, con su propia medición.
- B y C, por las razones de arriba.

## Cómo se mide

Tests que fallan hoy (la lista está en la [spec](spec.md)):

1. `longest_first` ordena de más a menos tokens y deja los empates en su orden. No compila hoy.
2. Una importación de otra herramienta y una de clawg mandan sus textos al embedder de más a menos tokens, en llamadas de a lo sumo 32. Hoy los mandan en el orden del archivo.
3. `count_all` da, para cada texto, lo mismo que `count`. No compila hoy.

Red de seguridad, que pasa hoy y tiene que seguir pasando: cada fila importada guarda el vector de su propio texto, cualquiera sea su lugar en el orden. Las pruebas de import que ya existen pasan sin cambiar ninguna aserción.

Medición: la importación real del mismo export, de punta a punta, con el build debug de la cabeza anterior al cambio y con el del cambio. Cada una corre en un `CLAWG_TEST_HOME` aislado del perfil dev, con un store vacío y una copia del modelo, sin otros clientes. El criterio, fijado antes de correr la del cambio:

- `embed` tarda al menos un 15 % menos que con el orden del archivo;
- el reporte es el mismo (`rows`, `inserted`, `by_lang`, `foreign`);
- la memoria privada con que termina `embed`, y su pico por minuto en `metrics.db`, no superan en más de un 5 % los de la corrida base.

Requiere el modelo real y el export de engram, fuera del repositorio, borrado al terminar.

## Migración y vuelta atrás

No hay esquema, contrato ni estado que cambien. Los vectores son los mismos bit por bit en la máquina medida. Volver es revertir los commits.
