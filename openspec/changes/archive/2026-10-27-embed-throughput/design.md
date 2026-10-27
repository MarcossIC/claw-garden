# embed-throughput — Diseño

Las mediciones son las de la [propuesta](proposal.md#problema): build debug, Windows x86_64, 16 CPUs lógicas, las 54.321 filas que el export de engram le da a un store vacío y, para el tiempo, las primeras 5.000.

## D1. Dónde se ordena: en quien llama

| Opción | A favor | En contra |
|---|---|---|
| **En la importación, antes del bucle de llamadas (elegida)** | Ve todas las filas a embeber. Ya guarda cada vector por el id de su fila (`vectors`, `crates/clawg-core/src/memory/portability/import.rs:1635`, `:1642`) y la escritura los busca por id, así que el orden no cambia qué vector va a qué fila. No toca el embedder ni su contrato | Cada llamador que embeba por tandas tiene que pedirlo (hoy sólo la importación; el re-embebido es fila por fila, `crates/clawg-core/src/memory/maintenance.rs:689`) |
| Dentro de `E5Embedder::embed_batch` | Lo ganaría cualquier llamador | La importación le pasa 32 textos por llamada (`import.rs:66`), una ventana en la que casi no hay nada que ordenar. Pasarle todas las filas juntas haría que una búsqueda esperara detrás de la llamada entera y no de una corrida de 16 (`embedder.rs:418-430`) |
| En el hilo del embedder | Tiene el tokenizer a mano | Las mismas dos objeciones, y además cambia la forma de `EmbedRequest` (`embedder.rs:405-408`) |

## D2. La clave: tokens

| Orden | Tokens corridos (M1) | Atención (M1) | 5.000 filas (M3) |
|---|---|---|---|
| archivo | 1,293x | 2,042x | 107,1 s |
| bytes del texto | 1,064x | 1,131x | 72,5 s (−32,3 %) |
| caracteres | 1,063x | 1,129x | 72,4 s (−32,3 %) |
| **tokens (elegido)** | 1,001x | 1,002x | 67,5 s (−37,0 %) |

Los bytes eran el candidato de la exploración «si el proxy alcanza». Alcanzan para la mayor parte de la ganancia, pero no para toda: la diferencia entre bytes y tokens se repite en las tres corridas (75,8/69,9, 72,5/67,3 y 72,4/67,5 s). Lo que cuesta contar los tokens de todas las filas, 5,4 s en debug, es menos de lo que gana en 5.000 filas. Los caracteres no mejoran a los bytes: la densidad de tokens que los desordena es la del idioma y del código, no la de los acentos.

## D3. Cómo se cuentan los tokens

| Opción | A favor | En contra |
|---|---|---|
| **El tokenizer del daemon por `ChatChunker`, en grupos, en un hilo bloqueante (elegida)** | Es el mismo tokenizer del embedder (`api.rs:1945` lo comparte con la celda que llena la carga del modelo), sin otra copia en memoria. `encode_batch` reparte cada grupo entre los núcleos, y un grupo de `COUNT_BATCH` textos mantiene chicas sus codificaciones. La importación de otra herramienta ya recibe el chunker | `import` gana un parámetro, como el que ya tiene `import_foreign` |
| `TokenSplitter::count` texto por texto | No agrega función | Un hilo solo: más lento que el grupo, que usa todos los núcleos |
| Llevar en `Row` los tokens que contó la traducción | No vuelve a tokenizar | Un export de clawg no se traduce, así que no los tiene. Además, la cabecera y la pieza no suman sus tokens (`crates/clawg-core/src/memory/chunker/mod.rs:42-44`) |
| Un método del trait `Embedder` que cuente | El embedder conoce su tokenizer | Agrega una obligación a cada implementación del trait para un solo llamador |

La cuenta es la del chunker: los tokens del texto sin el prefijo `query: ` ni los especiales, y sin el corte a 512. Para ordenar da lo mismo, porque el prefijo agrega lo mismo a cada fila y las filas de más de 512 se cortan todas al mismo largo. Sin tokenizer en la celda (no pasa en una importación real, que necesita el modelo cargado), la cuenta del chunker es un cuarto de los bytes y el orden queda por bytes.

## D4. La dirección: la más larga primero

| Opción | A favor | En contra |
|---|---|---|
| **De más a menos tokens (elegida)** | La corrida más cara en memoria, 16 × 512 tokens, va primero: si la máquina no la aguanta, la importación falla al empezar, antes de gastar la fase. El pico no cambia: hoy una fila de 480 tokens ya rellena hasta 480 las otras 15 de su corrida | Las filas/s suben a lo largo de la fase (ver 87 en la propuesta) |
| De menos a más | Las primeras llamadas son las rápidas | El pico llega al final, después de casi toda la fase |

Los empates quedan en el orden del archivo (`sort_by_key` es estable), así que el orden es una función de las filas y no del azar.

## D5. Lo que queda puro y lo que hace I/O

- `longest_first(tokens) -> Vec<usize>` es pura: los índices de mayor a menor cuenta, estable.
- `embedding_order(prepared, picked, chunker)` arma el texto de cada fila, lo cuenta con el chunker por grupos y devuelve `picked` en el orden de `longest_first`. Corre en `spawn_blocking`, porque tokenizar decenas de miles de textos no puede ocupar un hilo del runtime.
- `write` la llama dentro de la fase `embed`, así que la línea de log de `embed` incluye contar. Las llamadas, la tanda más lenta y las filas/s de esa línea se leen igual.

## D6. Lo que no se hace

- **B, solapar la tokenización con la inferencia:** tokenizar y rellenar es el 1,5 % del orden del archivo (M2), contra el ~10 % que lo justificaba.
- **C, INT8:** ver la decisión 7 de la propuesta.
- **El refresco de docs:** manda los pasajes de un archivo por llamada (`embed_passages`, `crates/clawg-core/src/memory/project_docs/mod.rs:464-489`), con su propio manejo de un vector inutilizable por documento. Adoptar `longest_first` ahí es otro cambio, con su propia medición.
