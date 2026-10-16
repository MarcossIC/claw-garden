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

## Diferencias entre los docs del cambio y el código

Encontradas al consolidar los requisitos contra `49bd269`, para el archivo:

1. La spec no lista `the_sweeper_ends_with_the_embedder` (`crates/clawg-core/src/memory/idle_embedder.rs:497`): `idle_embedder.rs` tiene 14 tests y la spec nombra 13. Va con R1.
2. R7 no tiene test. `the_cpu_provider_turns_its_arena_off` (`crates/clawg-core/src/memory/embedder.rs:713`) fija el arena de ONNX, no el `malloc_trim`: nada de la suite comprueba que el trim corra. La prueba es la medición.
3. El diseño (D1) y la propuesta citan `embedder.rs:333` para el trim. En 333 está la definición de `return_freed_memory`; la llamada está en `embedder.rs:316`, después de `drop(model)` en `:315`, y el `malloc_trim(0)` en `:339`.
4. La propuesta cita `DocIndexError::Query` en `crates/clawg-core/src/memory/project_docs/mod.rs:62`; la variante está en la 63 (la 62 es su atributo `#[error]`).
5. El procedimiento de Windows de este archivo pedía una búsqueda por texto, que no carga el modelo (ver «Cómo se midió»).

Sin diferencia: `should_unload` (`idle_embedder.rs:53`), `unloading_when_idle` (`bootstrap.rs:344`), `embed_failure` (`api.rs:860`), `embedder_resident` en `/health` (`api.rs:688`), el `wake` de `POST /v1/projects` (`api.rs:1188`), `hold_for_embedder` (`worker.rs:389`), `after_failure` (`outbox.rs:74-78`) y la `OnceCell` (`runtime.rs:107-115`) están donde dicen los docs. El barrido (un cuarto del ocio, entre 1 y 60 s) y `IDLE_MINUTES_VAR` también coinciden.

## Fuera de alcance

- Los tests del registro de `clawg path` compartían `HKCU\Software\clawg-test` como padre y cada uno lo borraba al terminar. Windows borra una clave que otro handle tiene abierta, así que un test que terminaba mientras otro creaba su clave debajo lo dejaba con un padre marcado para borrar: `create_subkey` fallaba con el error 1018, una vez cada ~30 corridas. Lo arregla `ce47e97`: cada test usa su propia clave bajo `Software`.
- Muchas citas `ruta:línea` de los specs de dominio y del backlog del README ya no apuntaban al símbolo que nombran, algunas desde antes de este cambio. Las corrige `9a78bbd`.
