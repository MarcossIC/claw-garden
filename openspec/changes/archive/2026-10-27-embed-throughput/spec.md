# embed-throughput — Especificación

Suma ET1 a ET3 a [`specs/portability`](../../../specs/portability/spec.md).

## ET1. Una importación embebe primero las filas de más tokens

La importación MUST mandar al embedder las filas que embebe (las que inserta y no están borradas) ordenadas por los tokens del texto que embebe cada una (`embedding_text` de su cabecera y su contenido), de más a menos. Dos filas con los mismos tokens MUST quedar en el orden del archivo. Los tokens MUST contarse con el tokenizer del daemon, el mismo del embedder, en un hilo bloqueante y en grupos, no todas las filas de una vez. Sin tokenizer, la cuenta MUST ser la del chunker sin él (un cuarto de los bytes, redondeado hacia arriba). Las llamadas al embedder MUST seguir siendo de a lo sumo `EMBED_BATCH` (32) textos. Vale igual para un export de clawg y para el de otra herramienta.

#### Escenario: el orden
- GIVEN filas de 3, 9, 3, 1 y 9 tokens, en ese orden
- WHEN se ordenan para embeber
- THEN el orden es la segunda, la quinta, la primera, la tercera y la cuarta

#### Escenario: una importación de otra herramienta
- GIVEN notas de largos mezclados, más que las de una llamada, con dos del mismo largo
- WHEN se importan con un embedder que registra cada llamada
- THEN los textos que recibe, una llamada después de otra, van de más a menos tokens
- AND ninguna llamada pasa de 32 textos
- AND las dos del mismo largo llegan en el orden del archivo

#### Escenario: un export de clawg
- GIVEN el export del store de prueba
- WHEN se importa con el mismo embedder
- THEN los textos que recibe van de más a menos tokens

#### Escenario: contar en grupo
- GIVEN un chunker con tokenizer y uno sin él
- WHEN cuentan varios textos juntos
- THEN dan, para cada texto, lo mismo que contarlo solo

**Tests:** `longest_first_puts_the_most_tokens_first_and_keeps_ties_in_order`, `an_import_of_another_tool_embeds_its_rows_longest_first` y `a_clawg_import_embeds_its_rows_longest_first` (`crates/clawg-core/src/memory/portability/import.rs`); `counting_together_counts_each_text_as_alone` (`crates/clawg-core/src/memory/chunker/token_splitter.rs`).

## ET2. El orden no cambia lo que se escribe

Cada fila importada MUST guardar el vector de su propio texto, cualquiera sea su lugar en el orden de ET1. Ni las filas que se escriben, ni sus ids, ni el reporte, ni el diario MUST cambiar con el orden. La escritura sigue por notas enteras, en el orden del archivo (`note_batches`).

#### Escenario: cada fila con su vector
- GIVEN notas de largos mezclados, con piezas de un guardado largo
- WHEN se importan con el embedder falso
- THEN el vector guardado de cada fila insertada y no borrada es el del embedder falso para el texto de su cabecera y su contenido

**Tests:** `each_imported_row_keeps_the_vector_of_its_own_text` (`import.rs`), y las pruebas de import de `import.rs`, `undo.rs`, `crates/clawg-cli/tests/api.rs` y `crates/clawg-cli/tests/binary.rs`, sin cambiar ninguna aserción.

## ET3. La medición de la fase de embeddings

Un test ignorado MUST medir la fase de embeddings sobre las filas que el export de otra herramienta le da a un store vacío. Con sólo el tokenizer, MUST decir cuántas veces los tokens reales y la atención corre el modelo en el orden del archivo, por bytes, por caracteres y por tokens, en las llamadas de la importación y las corridas del embedder. Con el modelo, sobre las primeras N filas, MUST decir cuánto tarda cada orden (la mediana de varias corridas alternadas), qué parte es tokenizar y rellenar y qué parte el pooling, y cuántos vectores cambian de bits entre órdenes y contra un texto embebido solo. Cada salida MUST decir el perfil de compilación, el sistema, las CPUs y sus extensiones. MUST NOT imprimir texto, ids ni rutas de las filas.

#### Escenario: la cuenta del relleno
- GIVEN corridas de largos conocidos
- WHEN se cuenta el relleno
- THEN cada corrida del embedder se rellena hasta su propio texto más largo, no hasta el de la llamada

**Tests:** `padding_counts_each_run_of_the_embedder_apart` y `the_embedding_phase_of_a_real_export` (`import.rs`, `#[ignore]`: necesita `CLAWG_E5_MODEL_DIR` y `CLAWG_IMPORT_EXPORT`, y opcionalmente `CLAWG_EMBED_ROWS` y `CLAWG_EMBED_RUNS`). Depende de la máquina, así que queda fuera de CI.
