# embedder-idle-unload — Verificación

## Linux

Sobre el último commit de la rama, una sola corrida (regla del proyecto):

- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- `cargo test --workspace`: **2.065 pasan, 0 fallan, 0 ignorados.** En `main` (`84ee27b`) pasaban 2.044; el cambio suma 21 tests: 14 en `idle_embedder.rs`, 1 en `bootstrap.rs`, 5 en `tests/api.rs` y 1 en `client.rs`.

**Paso 0** con el modelo real, cinco cargas y descargas en un proceso ([diseño, D1](design.md#d1-la-memoria-vuelve-medición)): tras soltar el modelo, 41-60 MB residentes en las cinco vueltas (659-762 MB sin el trim), y el mismo vector en todas.

`rag-eval` no aplica: los vectores son los mismos y no cambia el ranking.

## CI

- [x] El workflow en verde en `ubuntu-latest` y `windows-latest` en el PR #33 sobre `871cf9e`, el último commit con código (`9af2ad6` sólo toca instrucciones y el CI no corre), y en el merge `49bd269` en `main`.

## Windows, con el modelo real

Sobre `49bd269` (`main`, el merge del PR #33, versión 0.12.0), el 2026-10-16, build debug con `--target-dir target-test`:

- [x] `cargo test --workspace --target-dir target-test`: **2.071 pasan, 0 fallan, 0 ignorados**, la base de 2.050 más los 21 de este cambio (`c855be6` no suma tests: fuera de Linux con glibc el trim es una función vacía). La compilación no da ningún warning; `binary.rs`, 9 de 9 en 1,43 s. Una corrida anterior falló en `add_then_remove_round_trips_and_keeps_expand_sz`, una carrera de los tests del registro ajena a este cambio (ver «Fuera de alcance»).
- [x] `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- [x] Antes y después de la suite, los únicos `clawg.exe` vivos son los del editor (perfil prod), con los mismos pid.
- [ ] **La memoria vuelve en Windows: no se cumple.** Soltar y recargar el modelo funcionan, pero con el modelo suelto la memoria privada queda sobre 300 MB y crece con cada recarga.
- [x] Los tiempos de la recarga, anotados abajo. No deciden si saltear el SHA-256: en este build las dependencias no están optimizadas.

### Cómo se midió

El paso 1 del procedimiento original no carga el modelo: `clawg memories search` es la búsqueda por texto de `GET /v1/memories?q=` (`search_args`, `crates/clawg-cli/src/memories.rs:368`) y nunca embebe la consulta. Con ella, el modelo se soltó 60 s después del arranque y tres búsquedas seguidas no lo volvieron a cargar. La medición usa `POST /v1/search`, la búsqueda semántica de `search_memory`, con el token del perfil:

```
curl -X POST http://127.0.0.1:<puerto>/v1/search -H "Authorization: Bearer <token>" \
  -H 'Content-Type: application/json' \
  -d '{"query":"prueba de memoria del daemon","agent_id":"verify-windows","limit":3}'
```

Un daemon dev de este build, con `CLAWG_EMBEDDER_IDLE_MINUTES=1` en su entorno. En cada vuelta: una búsqueda semántica, una muestra de `PrivateMemorySize64` y de `embedder_resident` (`clawg status --profile dev`), dos minutos sin uso y otra muestra. El store dev tenía V13 y V14 pendientes: al abrirlo, el daemon escribió el snapshot `pre-migration` y las aplicó.

### Resultado

| Vuelta | Recarga en el log: archivos con hash + sesión | Privados con el modelo | Privados con el modelo suelto | Δ suelto |
|---|---|---|---|---|
| Arranque | — | 1.012 MB | 287,5 MB | — |
| 1 | 10.793 + 2.713 = 13.507 ms | 1.098 MB | 310,4 MB | +22,9 |
| 2 | 10.916 + 2.826 = 13.743 ms | 1.095 MB | 335,0 MB | +24,6 |
| 3 | 7.051 + 1.910 = 8.961 ms | 1.146 MB | 347,5 MB | +12,5 |
| 4 | 7.011 + 1.883 = 8.895 ms | 1.232 MB | 393,5 MB | +46,0 |
| 5 | 7.054 + 1.911 = 8.965 ms | 1.240 MB | 397,6 MB | +4,1 |

- **Soltar (R1):** cada descarga llega entre 60 y 75 s después del último uso (`embedding model unloaded after 60 s without use`), y `embedder_resident` pasa a `false`.
- **Recargar (R2, R4, R5):** cada búsqueda con el modelo suelto lo carga y responde `200`, y `embedder_resident` vuelve a `true`. La `distance` de cada hit es idéntica en las cinco vueltas: el vector de la consulta es el mismo. El `score` cambia porque cada hit mostrado refresca `last_accessed_at`.
- **`embedder`** sigue en `ready` en todas las muestras.
- **La memoria (R7) no vuelve lo suficiente.** El SHOULD de menos de 300 MB sólo se cumple tras la carga del arranque, y el MUST de no crecer falla: con el modelo suelto, la memoria privada sube 110 MB en cinco recargas y nunca baja. Con el modelo cargado también crece, de 1.012 a 1.240 MB. En las vueltas 4 y 5 el working set baja a 179 MB porque Windows recorta páginas, pero la memoria comprometida no se mueve.
- **Tiempos:** leer y hashear los archivos es el 78-80 % de la recarga. Las primeras dos recargas tardan ~13,6 s y las siguientes ~9 s. El workspace no tiene `[profile]`, así que en debug `sha2` compila sin optimizar y hashea los 465 MB a 43-66 MB/s. Si conviene saltear el SHA-256 en las recargas se decide midiendo con el binario de release.

### Causa

Un arnés descartable, fuera del repo, carga el modelo con `FastEmbedLocal::from_bytes` como el daemon (sin `ensure` ni `spawn_blocking`), embebe una consulta y 64 pasajes, lo suelta y mide `PrivateUsage` cuando se estabiliza. Corre 40 vueltas por variante, cada una en un proceso nuevo y en build debug:

- **El vector de la consulta es el mismo en las 41 cargas.**
- **No es una fuga de objetos vivos.** Los bytes ocupados del heap del proceso quedan en 8,4 MB de una vuelta a otra. Lo que crece es memoria libre que el heap de Windows mantiene comprometida: 87 MB en la vuelta 0 y 425 MB en la 20. Con las asignaciones de Rust en un heap propio, ese heap tiene 0,1 MB ocupados y su espacio libre sube de 78 a 432 MB en 20 vueltas; el lado de la UCRT y ONNX Runtime crece ~1,3 MB por vuelta.
- **Crece sin cota a la vista.** La memoria privada tras soltar el modelo alterna entre ~55-60 MB y saltos de ~300 MB que siguen a lo comprometido; en la vuelta 40 está en 518 MB y sigue subiendo.
- **`HeapCompact` no lo arregla**, ni sobre el heap del proceso ni sobre todos los heaps: termina en 506 y 478 MB contra 518, y devuelve 1-2,5 MB como mucho. Con mimalloc como asignador de Rust (una corrida de 30 vueltas), la memoria tras soltar queda plana, pero entre 280 y 430 MB.

**Lo que crece es el tokenizer.** Mismo arnés, cada variante en un proceso nuevo:

| Variante | Privados tras soltar | ¿Crece? |
|---|---|---|
| T: sólo el tokenizer, armado como lo arma fastembed, 40 vueltas | 12,5 → 481 MB, con el mismo patrón de saltos | Sí |
| S: sólo la sesión de ONNX Runtime, con las opciones de clawg, 40 vueltas | 19-25 MB | No |
| K: el tokenizer se parsea una vez y queda; la sesión se arma y se suelta en cada vuelta, 40 vueltas | 330-384 MB | No |
| M: mimalloc como asignador de Rust y `mi_collect(true)` tras soltar, 30 vueltas | 117-132 MB | No |
| E: el camino de recarga del daemon (`ensure_model` con SHA-256 y `spawn_blocking`), 15 vueltas | igual que la carga completa | Sí |

Cada parse del tokenizer son ~1,2 millones de asignaciones chicas, y lo que dejan libre el heap de Windows no lo devuelve. En K el vector es idéntico al de fastembed (diferencia máxima 0,0), aunque la inferencia la arma el arnés: fastembed no deja construir un `TextEmbedding` con un tokenizer ya parseado.

**El piso del daemon es otro tokenizer.** El daemon tiene un tokenizer propio para el chunker y para contar los tokens de las consultas (`resolve_tokenizer_and_chunker`, `crates/clawg-core/src/memory/worker.rs:809`, que carga `load_tokenizer`, `crates/clawg-core/src/memory/model_download.rs:133`), y lo guarda en una `OnceCell` durante toda la vida del proceso. Es otro parse del mismo `tokenizer.json`, aparte del que fastembed arma adentro del modelo. Con el embedder falso, en un home temporal:

| Daemon | Privados |
|---|---|
| Sin `tokenizer.json` | 6-9 MB |
| Sólo con `tokenizer.json` | 262-265 MB |

Los 287 MB tras la primera descarga son ese tokenizer más ~20 MB. R7 pide menos de 300 MB tomando la medición de un arnés que no tiene ese tokenizer: en el daemon, en cualquier plataforma, el piso con el modelo suelto ronda los 265 MB en este build. Con el modelo cargado, el daemon tiene dos parses del mismo tokenizer.

## Después de D5: un solo tokenizer

Sobre `71fd1fa`, el cambio de motor (`269de1f` sólo suma docs), el 2026-10-16, en Windows, build debug con `--target-dir target-test`:

- [x] `cargo test --workspace --target-dir target-test`: **2.119 pasan, 0 fallan, 1 ignorado** (`the_vectors_are_the_ones_fastembed_gave`, que necesita el modelo), sin warnings. `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- [x] **Los mismos vectores (R9).** `cargo test -p clawg-core --features eval --test e5_golden -- --include-ignored` con `CLAWG_E5_MODEL_DIR`: los 31 textos de la fixture, generada con fastembed en `9112ab3`, dan los mismos bits con `E5Embedder`, en la CPU que la generó (x86_64 con AVX2, AVX-512F y FMA). Sin la variable, el test falla.
- [x] El lock pierde `fastembed`, `safetensors`, `allocator-api2` y `foldhash` 0.2.0, y no suma ningún paquete.
- [x] **`rag-eval --diff --real`** de `cda7ead` (el último commit con fastembed, armado en un worktree) a `269de1f`, sobre la golden set de 46 casos: ningún caso se mueve en ninguna métrica ni en ningún corte por par de idiomas, y con `--agent-view` tampoco (`search.found` y `context.found` incluidos). Las dos comparaciones salen con código 0 bajo `--fail-on-regression`.
- [x] **R7 en el daemon.** Se cumple: abajo.
- [x] **CI en Linux y Windows.** El workflow pasa clippy y `cargo test --workspace` en `ubuntu-latest` y `windows-latest` sobre `0d36f88`, el último commit con código. El primer run con el cambio de motor, sobre `1233d13`, pasó en Linux y falló en el `cargo test` de Windows; desde `d095d84`, que cierra el store de `rag-eval --agent-view` antes de borrar su directorio temporal, pasa en los dos.
- [x] `cargo test --workspace --target-dir target-test` en Windows sobre `d095d84` (0.13.0), con los artefactos del workspace limpiados antes: **2.120 pasan, 0 fallan, 1 ignorado**, el de `e5_golden`.
- [ ] **R7 en Linux: no se cumple.** La mediana con el modelo suelto queda 75,9 MB sobre la línea de base y la segunda mitad 18,4 MB sobre la primera: abajo, en «R7 en Linux». El cambio vuelve a diseño.

### R7 en Windows

**Línea de base:** el mismo build con `CLAWG_TEST_EMBEDDER=fake` y sólo `tokenizer.json`, en un home temporal: **264,2 MB**, con una línea `tokenizer.json parsed`.

**Con el modelo:** el daemon dev de este build con `CLAWG_EMBEDDER_IDLE_MINUTES=1`. Se soltó primero el modelo del arranque (282,6 MB) y después hubo diez recargas, cada una con una búsqueda semántica (`POST /v1/search`) y 100 s sin uso:

| Recarga | En el log: archivos con hash + sesión | Privados con el modelo | Privados suelto |
|---|---|---|---|
| 1 | 8.665 + 217 ms | 739,4 MB | 285,4 MB |
| 2 | 12.641 + 458 ms | 743,4 MB | 284,8 MB |
| 3 | 8.679 + 195 ms | 745,8 MB | 284,0 MB |
| 4 | 13.517 + 512 ms | 741,8 MB | 284,7 MB |
| 5 | 12.253 + 433 ms | 745,8 MB | 285,6 MB |
| 6 | 8.961 + 247 ms | 745,3 MB | 286,3 MB |
| 7 | 12.114 + 441 ms | 746,7 MB | 285,3 MB |
| 8 | 11.704 + 472 ms | 746,4 MB | 288,5 MB |
| 9 | 11.756 + 319 ms | 747,3 MB | 289,8 MB |
| 10 | 11.654 + 309 ms | 749,2 MB | 287,2 MB |

- **Mediana de las diez:** 285,5 MB, **21,3 MB sobre la línea de base** (el límite es 64).
- **Medianas de 1 a 5 y de 6 a 10:** 284,8 y 287,2 MB, **+2,4 MB** (el límite es 16).
- **Parses:** el log tiene una sola línea `tokenizer.json parsed`, la del arranque.
- **Vectores:** la `distance` de cada hit es la misma en las diez recargas, 0,572561502456665 y 0,692162275314331, y es la que daba fastembed antes del cambio.
- **Contra la primera implementación:** con el modelo, ~745 MB en lugar de 1.095-1.240 MB, porque falta el segundo parse del tokenizer. Suelto, plano en 284-290 MB en lugar de subir de 310 a 398 MB. Armar la sesión tarda 0,2-0,5 s en lugar de 1,9-2,8 s, porque ya no se parsea el tokenizer. La lectura con hash sigue siendo lo que más tarda: 8,7-13,5 s en debug.

### R7 en Linux

Sobre `cd96f34` (0.13.0), el 2026-10-17, en Linux x86_64 con glibc (Intel Xeon a 2,10 GHz, 4 hilos, AVX2, AVX-512F y FMA), con el build debug de `cargo test --workspace` en `target`. Sobre ese mismo commit, `cargo test --workspace` da **2.113 pasan, 0 fallan, 1 ignorado** (el de `e5_golden`) y `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` sale con 0. Los cinco archivos del modelo, bajados de `HF_BASE_URL`, pasan el SHA-256 de `ALL_FILES`. Las dos corridas usan un `CLAWG_TEST_HOME` temporal, así que ningún perfil real se toca; la muestra es `RssAnon` de `/proc/<pid>/status`, y la estable, 8 lecturas cada 0,5 s dentro de 1 MB.

**Línea de base:** `CLAWG_TEST_EMBEDDER=fake` y sólo `tokenizer.json`: **285,0 MB**, con una línea `tokenizer.json parsed`.

**Con el modelo:** `CLAWG_EMBEDDER_IDLE_MINUTES=1` y dos memorias guardadas antes de empezar, para que la búsqueda tenga hits. Se soltó primero el modelo del arranque (298,0 MB) y después hubo diez recargas, cada una con un `POST /v1/search` y la espera de `embedder_resident` en `false`:

| Recarga | En el log: archivos con hash + sesión | `RssAnon` con el modelo | `RssAnon` suelto |
|---|---|---|---|
| 1 | 12.015 + 4.059 ms | 822,7 MB | 313,4 MB |
| 2 | 13.749 + 2.981 ms | 795,9 MB | 344,7 MB |
| 3 | 11.824 + 1.961 ms | 847,1 MB | 344,6 MB |
| 4 | 11.940 + 2.378 ms | 796,0 MB | 344,8 MB |
| 5 | 12.254 + 1.384 ms | 863,4 MB | 360,9 MB |
| 6 | 12.039 + 1.878 ms | 812,2 MB | 361,1 MB |
| 7 | 11.971 + 1.831 ms | 863,4 MB | 360,9 MB |
| 8 | 12.428 + 1.988 ms | 814,4 MB | 363,3 MB |
| 9 | 12.034 + 2.110 ms | 865,6 MB | 363,1 MB |
| 10 | 12.050 + 2.139 ms | 814,4 MB | 363,3 MB |

- **Mediana de las diez:** 360,9 MB, **75,9 MB sobre la línea de base** (el límite es 64). **No cumple.**
- **Medianas de 1 a 5 y de 6 a 10:** 344,7 y 363,1 MB, **+18,4 MB** (el límite es 16). **No cumple.**
- **Parses:** una sola línea `tokenizer.json parsed`, la del arranque. Cumple.
- **Vectores:** la `distance` de los dos hits es la misma en las diez recargas, 0,5915524959564209 y 0,6260557770729065. Cumple.
- **La forma:** no crece en cada vuelta como fastembed en Windows, sino a escalones que se aplanan: +15 MB en la primera recarga, +31 en la segunda, +16 en la quinta y +2 en la octava, y entre escalones queda quieto. Con el modelo cargado no hay tendencia (796-866 MB).
- **Hipótesis para el diseño, sin probar:** cada carga arma la sesión y su hilo `embedder-worker` nuevos, y glibc le da a cada hilo nuevo una arena de `malloc` propia, hasta 8 por núcleo; `malloc_trim(0)` devuelve las páginas libres enteras, pero lo que queda ocupado o fragmentado en cada arena no vuelve. Escalones que se aplanan encajan con un número acotado de arenas. Lo que conviene medir primero: `M_ARENA_MAX` (o `MALLOC_ARENA_MAX`) bajo, y un solo hilo dueño de la sesión que viva lo que vive el proceso.

### Por qué no cumple: las arenas de glibc

Misma máquina y mismo build, el 2026-10-17, tres corridas de diez recargas en paralelo con el procedimiento de arriba. En cada muestra también se desglosó `/proc/<pid>/smaps`: el `[heap]` de la arena principal y las reservas anónimas alineadas a 64 MB, que son los heaps de las otras arenas. Las líneas de base, con y sin `MALLOC_ARENA_MAX=1`, dan 284,6 y 285,0 MB.

| Variante | Mediana suelta | Sobre la base | 6 a 10 contra 1 a 5 | Recarga: hash + sesión |
|---|---|---|---|---|
| Por defecto | 345,4 MB | +60,4 | +18,3 | 11,7-12,4 s + 2,1-3,8 s |
| `MALLOC_ARENA_MAX=2` | 327,5 MB | +42,5 | +1,2 | 12,0-12,5 s + 1,8-3,7 s |
| `MALLOC_ARENA_MAX=1` | 292,2 MB | **+7,2** | **0,0** | 11,8-12,6 s + 2,0-4,0 s |

- **Por defecto**, cada escalón es una arena que retiene de 6 a 24 MB que `malloc_trim(0)` no devuelve: a la vuelta 4 hay cinco arenas así (23,9, 17,5, 16,9, 6,1 y 3,8 MB), y desde ahí nada cambia. La sesión se arma en un hilo del pool de bloqueo de tokio (`spawn_blocking` en `unloading_when_idle`), la inferencia corre en un `embedder-worker` nuevo en cada carga y los hilos intra-op de ONNX Runtime también son nuevos, así que cada recarga puede caer en otra arena hasta llenar el tope de glibc (8 por núcleo). Entre esta corrida (+60,4) y la primera (+75,9) se ve que el resultado depende de qué arenas toque cada vuelta.
- **Con una sola arena**, la memoria suelta queda en 292,1-292,2 MB en las diez vueltas, sin escalones.
- Las `distance` son las mismas en las tres variantes y en las diez vueltas, y hay un solo parse del tokenizer en cada log.
- **El costo en la inferencia no se ve:** el test `e5_golden`, que embebe 32 textos (entre ellos uno de ~2.000 tokens y un lote de 17), tarda 19,2-20,1 s con `MALLOC_ARENA_MAX=1` y 19,5-19,8 s sin él, en tres pares alternados.

La propuesta que sale de esto es [D6](design.md#d6-una-sola-arena-de-malloc-en-linux-propuesta).

### Los vectores en Linux (R9)

Con el mismo modelo, `CLAWG_E5_MODEL_DIR=<dir> cargo test -p clawg-core --features eval --test e5_golden -- --include-ignored` **falla**: la huella de esta CPU (`x86_64`, AVX2, AVX-512F y FMA) es la de la fixture, así que el test compara bits, y los 32 vectores difieren en los últimos bits, el que más por 8,9e-8. Comparados con la tolerancia de 1e-6, pasan los 32.

- No es el número de hilos: con `taskset` a 1, 2 y 4 núcleos los 32 vectores dan los mismos bits.
- Para fp32, ONNX Runtime elige sus kernels por AVX-512F y por AVX2 con FMA, que coinciden; las extensiones que esta CPU suma (VNNI, AMX, FP16) son de los kernels de enteros. Lo que cambia es el sistema: la fixture se generó en Windows, y ONNX Runtime viene compilado de otra forma en cada uno. No está aislado entre las dos causas.
- R9 dice «los mismos bits en una CPU como la que generó la referencia», y el test decide qué es «como» con una huella que no incluye el sistema. Hay que decidir si la huella suma el sistema operativo (`std::env::consts::OS`) o si R9 promete bits exactos sólo en la máquina que generó la fixture. Los vectores en sí están dentro de lo que R9 permite en otra máquina.

## Para archivar

**Estado al 2026-10-17:** R7 en Linux no se cumple (arriba), así que, por el punto 4, el cambio vuelve a diseño y no se archiva. El diseño nuevo también tiene que resolver la huella de `e5_golden` («Los vectores en Linux»).

Sólo falta R7 en Linux; lo demás está hecho y verificado. Quien lo mida no necesita otra revisión para archivar:

1. **Medir.** En una máquina Linux x86_64 con glibc, el procedimiento de [D5, «R7 sobre lo medido»](design.md#r7-sobre-lo-medido) con el build de `cargo test --workspace` (en Linux el directorio es `target`), muestreando `RssAnon` de `/proc/<pid>/status`. Las dos corridas usan un `CLAWG_TEST_HOME` temporal, así que ningún perfil real se toca.
2. **Anotar.** Una sección «R7 en Linux» como «R7 en Windows», con la línea de base y la tabla de las diez recargas, y marcar la última tarea de la sección 10 de [tasks](tasks.md) y su línea en «Después de D5».
3. **Si cumple** (mediana a no más de 64 MB de la línea de base, no más de 16 MB entre la mediana de 1 a 5 y la de 6 a 10, una sola línea `tokenizer.json parsed` y la misma `distance` en las diez), archivar: consolidar R1 a R9 donde dice el encabezado de la [spec](spec.md), incluido el requisito «El modelo se carga con pooling mean» de `specs/storage`, que todavía nombra `FastEmbedLocal` y `the_e5_model_is_pooled_by_mean`; resolver ahí las diferencias de abajo; mover el cambio a `changes/archive/` con la fecha; actualizar sus dos filas en `openspec/README.md` (el mapa y el orden); y subir la versión menor (`.claude/CLAUDE.md`, «Versioning»).
4. **Si no cumple**, el cambio vuelve a diseño y no se archiva.

## Diferencias entre los docs del cambio y el código

Encontradas al consolidar los requisitos contra `49bd269`, para el archivo:

1. La spec no lista `the_sweeper_ends_with_the_embedder` (`crates/clawg-core/src/memory/idle_embedder.rs:497`): `idle_embedder.rs` tiene 14 tests y la spec nombra 13. Va con R1.
2. R7 no tiene test. `the_cpu_provider_turns_its_arena_off` (`crates/clawg-core/src/memory/embedder.rs:713`) fija el arena de ONNX, no el `malloc_trim`: nada de la suite comprueba que el trim corra. La prueba es la medición.
3. El diseño (D1) y la propuesta citan `embedder.rs:333` para el trim. En 333 está la definición de `return_freed_memory`; la llamada está en `embedder.rs:316`, después de `drop(model)` en `:315`, y el `malloc_trim(0)` en `:339`.
4. La propuesta cita `DocIndexError::Query` en `crates/clawg-core/src/memory/project_docs/mod.rs:62`; la variante está en la 63 (la 62 es su atributo `#[error]`).
5. El procedimiento de Windows de este archivo pedía una búsqueda por texto, que no carga el modelo (ver «Cómo se midió»).
6. R8 en la spec ubica `the_worker_and_the_model_load_share_one_parse` en `worker.rs`; está en `bootstrap.rs`.

Sin diferencia: `should_unload` (`idle_embedder.rs:53`), `unloading_when_idle` (`bootstrap.rs:344`), `embed_failure` (`api.rs:860`), `embedder_resident` en `/health` (`api.rs:688`), el `wake` de `POST /v1/projects` (`api.rs:1188`), `hold_for_embedder` (`worker.rs:389`), `after_failure` (`outbox.rs:74-78`) y la `OnceCell` (`runtime.rs:107-115`) están donde dicen los docs. El barrido (un cuarto del ocio, entre 1 y 60 s) y `IDLE_MINUTES_VAR` también coinciden.

Contra `0d36f88`, después de D5, se movieron: `unloading_when_idle` a `bootstrap.rs:381`; `the_cpu_provider_turns_its_arena_off` a `embedder.rs:1223`; y el trim, que ahora sigue a `drop(inference.session)` (`embedder.rs:554`), con la llamada en `:555`, la definición de `return_freed_memory` en `:572` y el `malloc_trim(0)` en `:578`. Lo de R8, para consolidarlo: la celda es `MemoryRuntime::tokenizer` (`crates/clawg-core/src/runtime.rs:124`); la llenan `resolve_tokenizer_and_chunker` (`worker.rs:810`), con `load_verified_tokenizer` (`model_download.rs:154`), y `model_loader` (`bootstrap.rs:228`), y cada parse deja la línea `tokenizer.json parsed` (`model_download.rs:164` y `:191`). El resto de las citas de arriba sigue valiendo.

## Fuera de alcance

- Los tests del registro de `clawg path` compartían `HKCU\Software\clawg-test` como padre y cada uno lo borraba al terminar. Windows borra una clave que otro handle tiene abierta, así que un test que terminaba mientras otro creaba su clave debajo lo dejaba con un padre marcado para borrar: `create_subkey` fallaba con el error 1018, una vez cada ~30 corridas. Lo arregla `ce47e97`: cada test usa su propia clave bajo `Software`.
- Muchas citas `ruta:línea` de los specs de dominio y del backlog del README ya no apuntaban al símbolo que nombran, algunas desde antes de este cambio. Las corrige `9a78bbd`.
