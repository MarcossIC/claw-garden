# quality-guards — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S | — (conviene que cierre antes de cualquier cambio que toque la superficie MCP: 22, 23, 25, 27) | exploración (borrador) |

**Tesis:** lo que el agente ve (nombres, descripciones, schemas e instrucciones de las herramientas) es el producto, y hoy puede cambiar en cualquier commit sin que ningún test lo note. Lo mismo pasa con los docs que enumeran esa superficie y con la regla de "no dejar código muerto". Son tres redes baratas, de tests y de documentación, que hacen que un cambio deliberado se vea como diff y uno accidental falle.

## Problema (verificado contra el código)

### 1. La superficie para el agente no está fijada

- Las instrucciones del servidor son una constante, `INSTRUCTIONS` (`crates/clawg-cli/src/mcp/server.rs:108-119`). Se devuelven en `get_info` (`server.rs:973-983`).
- Hay once herramientas, cada una con `#[tool(` (`server.rs:785` a `:957`).
- **Qué fijan los tests hoy:**
  - Los **nombres y el conteo**, en tres lugares: `every_tool_is_registered_under_its_wire_name` (`server.rs:1497-1521`, `names.len() == 11`), `the_viewer_is_not_exposed_to_the_model` (`server.rs:1528-1549`, la lista ordenada completa) y dos tests de `handshake.rs` (`:385`, `:442`).
  - Las **cinco propiedades** de `list_memories` (`server.rs:1561`).
  - Que el schema de `save_memory` **mencione** los siete tipos de memoria (`server.rs:1569-1589`) y que ofrezca `sources` (`:1593`).
  - Que las instrucciones **contengan** `save_memory`, `synthesize`, `search_project`, `how` y `documentation and code` (`server.rs:1471-1494`).
- **Qué no fija nadie:** el texto de la descripción de cada herramienta, el resto de los schemas (nombres, tipos, `required`, descripciones de campo), y el texto exacto de las instrucciones. Se puede reescribir la descripción de `delete_memory`, quitar un campo opcional de `search_memory` o acortar las instrucciones, y `cargo test` pasa.
- Es un riesgo real porque esa superficie se paga en el contexto de **cada** sesión del cliente y orienta cuándo el modelo llama a cada herramienta. Los cambios planeados 22, 23, 25 y 27 la tocan a propósito.
- **Un matiz sobre `rmcp`:** los schemas los genera `schemars` a partir de los `JsonSchema` de los argumentos (`server.rs:122`, `:168`). Una actualización de `rmcp` o `schemars` puede cambiar el JSON sin tocar una línea nuestra. Un test de snapshot lo mostraría; es una ventaja (nos enteramos) y un costo (hay que revisar y bendecir).

### 2. Los docs que enumeran la superficie se atrasan

Sólo lo verificado:

| Doc | Qué dice | Qué es cierto |
|---|---|---|
| `openspec/specs/mcp/spec.md:46`, `:59`, `:65` | "once herramientas", con la lista completa, incluida `search_project` | **La premisa del pedido es falsa:** esta spec ya dice once y ya incluye `search_project`. No hay nada que corregir ahí |
| `.claude/CLAUDE.md:52` | Lista las once por nombre | Coincide con `server.rs` |
| `AGENTS.md:39` | Lista **siete**: `save_memory`, `search_memory`, `recall_context`, `list_memories`, `delete_memory`, `resolve_conflict`, `memory_status` | **Atrasado.** Faltan `search_project`, `read_memory`, `update_memory` y `synthesize` |
| `AGENTS.md:37` | La API HTTP sin `GET .../save`, `POST /v1/projects`, `POST /v1/projects/docs/search`, `POST /v1/backup`, `POST /v1/synthesis`, y sin el sharing por `sharing.json` | **Atrasado** respecto de `.claude/CLAUDE.md:44` |
| `.claude/CLAUDE.md` (sección Commands) | No tiene `clawg memories`, que existe (`cli.rs:116`, `main.rs:188`) | **Falta.** Ya lo anotó recoverable-delete |
| `openspec/specs/storage/spec.md:225` | `AUTO_KEEP` en `backup.rs:226` | Es `backup.rs:230`. Desfase de referencia, sin consecuencia de comportamiento |
| `crates/clawg-core/src/memory/backup.rs:227` (comentario de `AUTO_KEEP`) | "automatic snapshots (`pre-migration`, `pre-restore`)" | Atrasado: `is_automatic` también cubre `pre-reembed` (`backup.rs:143-145`) |
| `openspec/README.md:130` | "1304 tests" en una fila de ideas evaluadas | La línea base de hoy es 1864 (`.claude/CLAUDE.md:84`, `config.yaml:21`). Es texto histórico de una fila ya resuelta: se corrige o se deja con su fecha |
| `openspec/README.md`, fila 12 | Se pidió verificar valores viejos | **No encontré ninguno.** `RELEVANCE_RATIO` 0,35 coincide con `relevance.rs:19`, `specs/retrieval` y `config.yaml:19` |
| `README.md:110`, `:143` | «por ejemplo `clawg 0.2.0`» | La versión del workspace es 0.4.0 (`Cargo.toml:6`). Es un ejemplo, pero un número que envejece en cada bump. Se cura sin número («imprime la versión instalada») |
| `AGENTS.md:71` | «Current baseline: 1304 tests» | La línea base es 1864 (`.claude/CLAUDE.md`, Testing). Mismo origen que el atraso de herramientas: `AGENTS.md` es una copia vieja |

`AGENTS.md` y `.claude/CLAUDE.md` se parecen casi párrafo por párrafo: el primero es una versión vieja del segundo. Mantener dos copias es la causa del atraso.

### 3. Código muerto contra la convención

La convención del repo: "no `#[allow]` to hide dead code" (`.claude/CLAUDE.md`, Code conventions).

`rg '#\[allow\(dead_code\)\]'` sobre `crates/` encuentra **tres**, no dos:

| Dónde | Qué es | Veredicto |
|---|---|---|
| `crates/clawg-core/src/memory/clock.rs:99` | `TimestampMs::age_days_from`, "kept ahead of its caller" | Código especulativo. Sólo lo usan sus tests; el puntaje no lo usa a propósito (README, backlog). **Se borra con sus tests** |
| `crates/clawg-eval/src/bin/rag-eval/golden.rs:167` | `RefMap::id_of`, sólo para los tests de `golden.rs` y `seed.rs` | `#[cfg(test)]` en lugar del `allow` (README, backlog) |
| `crates/clawg-core/src/memory/chunker/mod.rs:90` | El campo `Chunk::token_count`: "Nothing reads it yet" | **No está en el backlog del README.** `rg token_count` muestra que lo **escriben** `token_splitter.rs:99`, `:126`, `project_docs/mod.rs:1010`, `:1054` y `worker.rs:2345`, y lo **leen** sólo los tests (`token_splitter.rs:760`, `:822`). Quitarlo toca esos sitios de construcción |

Y hay un warning real en `cargo test --workspace`: `insert_row_with_embedding` (`crates/clawg-core/src/memory/worker.rs:1280`). Es un helper de test sin llamador: `rg` encuentra sólo su definición. El número de línea del backlog (`:1247`) quedó viejo.

Aparte, hay dos `#[allow(clippy::too_many_arguments)]` (`bootstrap.rs:463`, `prompt_builder.rs:1296`). No son código muerto y esta exploración no los toca.

### 4. No hay ninguna compuerta

- No existe `.github/`. Tampoco `clippy.toml`, `rustfmt.toml`, `deny.toml`, `rust-toolchain`, `justfile`, `Makefile` ni `.cargo/config`. Ningún `Cargo.toml` ni raíz de crate declara `[lints]`, `#![deny(...)]` ni `#![forbid(...)]` (`rg` de `lints|deny|forbid|warn` no encuentra nada).
- El `origin` es GitHub (`git remote -v`), así que un workflow es posible, pero es infraestructura nueva y es decisión del usuario.
- El único gate que corre siempre es `cargo test --workspace` (strict TDD: cada commit pasa la suite). Los dos `allow(clippy::…)` sugieren que alguna vez se corrió `clippy`; **no puedo afirmar que hoy esté limpio**, porque no corrí nada. Sin `-D warnings`, un warning de `dead_code` no rompe nada, que es como llegó `insert_row_with_embedding`.
- `dead_code` es un lint de `rustc`, no de `clippy`: `cargo test` ya lo emite. Lo que falta no es el análisis, es hacerlo fallar.

### 5. Lo opcional: latencia y `selftest`

- **Latencia.** `rag-eval --agent-view` ya calcula `latency_p50_ms` y `latency_p99_ms` por herramienta (`crates/clawg-eval/src/bin/rag-eval/agent_view/stats.rs:21-22`, `:58-59`). Pero depende del modelo real (`--real`) y de la máquina, y `--diff` no compara agent-view (README, backlog). Un umbral fijo de latencia sería frágil entre equipos.
- **`selftest`.** Los tests de integración ya levantan el daemon y el puente en el proceso (`crates/clawg-cli/tests/api.rs`). **La premisa "Linux y macOS sin probar" es parcialmente falsa:** Linux está verificado en los `verify.md` archivados (README, filas 1b, 6, 9, 10 y siguientes). Lo que no está probado es macOS: el README del proyecto dice que el script y `clawg path` son sólo de Windows y manda a compilar a mano (`README.md:131`), y no hay una sola mención de macOS en código o specs fuera de un ejemplo de Markdown. `cfg(windows)` aparece en `process.rs`, `user_path.rs`, `client.rs`, `main.rs`, `lifecycle.rs` y `doctor.rs`.

## Lo que ya existe y se reutiliza

- `bridge()` y `bridge_at` en el `mod tests` de `server.rs` (`:1013-1025`) arman un puente completo sin proceso aparte. `tool_router.list_all()` ya devuelve nombre, descripción y `input_schema` de cada herramienta.
- `get_info().instructions` ya expone el texto.
- `tempfile`, `serde_json` y `std::fs` ya están. El snapshot y el escaneo de fuentes no necesitan una dependencia nueva (regla de `config.yaml`: sin dependencia sin uso real).
- Los tests que fijan nombres y conteo ya existen; el snapshot los absorbe, pero no hace falta borrarlos en este cambio.
- `include_str!` con ruta relativa al archivo: sirve para un JSON dentro del crate. Para leer `.claude/CLAUDE.md` hay que subir fuera del crate (`../../../../.claude/CLAUDE.md` desde `crates/clawg-cli/src/mcp/`), lo que funciona en este árbol y no al empaquetar el crate. Estos crates no se publican; verificar `publish` antes de decidirlo.

## Enfoques posibles

### A. Snapshot versionado del contrato del agente

Un JSON dentro del repo (por ejemplo `crates/clawg-cli/tests/agent-contract.json` o junto a `server.rs`) con, por herramienta ordenada por nombre: `name`, `description`, `input_schema`; y el texto de `INSTRUCTIONS`. Un test arma lo mismo desde el puente y compara.

- **Qué excluir:** `serverInfo.version` (cambia en cada bump de versión, `CLAUDE.md` Versioning) y el nombre del servidor (depende del perfil, `server.rs:1475`). Sólo lo que el modelo lee para decidir.
- **Cómo se bendice un cambio:** un test que, si existe `CLAWG_BLESS=1`, **reescribe** el archivo en lugar de comparar. El diff del JSON queda en el commit y se revisa como código. Alternativa: un test `#[ignore]` que regenera (`cargo test -- --ignored regenerate_contract`). Ambas dejan el archivo a la vista.
- A favor: cambia lo que hoy es invisible por un diff; cubre schemas generados; los cambios 22 (instrucciones cortas), 23, 25 y 27 muestran su efecto en la superficie sin escribir un test nuevo por campo.
- En contra: **ruido** cuando se toca una descripción a propósito (es el punto); churn si `schemars` reordena claves (se mitiga serializando con claves ordenadas, `serde_json::Value` ya ordena con `BTreeMap` salvo que esté `preserve_order`; **a verificar**); el JSON es largo de leer.
- Riesgo de la variable de entorno: una corrida con `CLAWG_BLESS=1` puesta en el shell "bendice" todo sin darse cuenta. Mitigación: que el test escriba y **falle** igual con "actualizado, volvé a correr", así nunca pasa en verde bendiciendo.

### B. Inventario de herramientas contra los docs que las enumeran

Un test lee `.claude/CLAUDE.md` (y, según decisión, `AGENTS.md` y `openspec/specs/mcp/spec.md`) con `include_str!`, extrae los nombres entre comillas invertidas de la línea `MCP tools (...)` y compara el conjunto con `tool_router.list_all()`.

- A favor: atrapa exactamente el atraso de `AGENTS.md:39`.
- En contra: **un test que lee Markdown** es frágil a la redacción. Se limita a comparar el **conjunto de nombres** de una línea con un prefijo estable, no frases ni el conteo en palabras ("once"). La spec en español repite los nombres dentro de una oración (`mcp/spec.md:59`): extraerlos es más frágil que en `CLAUDE.md`.
- Recomendación: sólo `.claude/CLAUDE.md` (formato estable, línea propia). `AGENTS.md`, o se cura con un cambio de docs o se reemplaza (decisión abierta). La spec ya la mantiene el flujo de archivo de openspec.

### C. Código muerto: sacar los `allow` y poner una compuerta

1. Sacar los tres `allow(dead_code)` y el helper huérfano (cada uno como el backlog dice; `token_count` es nuevo y necesita decisión: borrar el campo o dejarlo con un lector real).
2. Una compuerta, con tres lugares posibles dado que no hay CI:

| Compuerta | Dónde corre | Costo | Notas |
|---|---|---|---|
| **a. Un test que falla si aparece `#[allow(dead_code)]` en `crates/*/src`** | `cargo test --workspace`, que ya corre en cada commit | Cero dependencias: `std::fs` recorriendo `CARGO_MANIFEST_DIR/..` | Garantiza la **convención escrita**, no el warning. Es un test que lee fuentes (misma salvedad que B). No ve un `dead_code` que el compilador marque sin `allow` |
| **b. `cargo clippy --workspace --all-targets -- -D warnings`** documentado en `.claude/CLAUDE.md` (Testing) | A mano, antes de commitear | Cero código | Cubre `dead_code` y todos los demás warnings. Sin CI depende de la disciplina. En Windows hay que usar `--target-dir` aparte por el lock de `clawg.exe` (`.claude/CLAUDE.md`, Testing). **Sin verificar que hoy pase limpio** |
| **c. Un workflow de GitHub Actions** con (b) | En cada push | Infraestructura nueva (matriz Linux y Windows) | Es la única que corre sin que alguien se acuerde, y la única que prueba Linux por sí sola. Decisión del usuario; fuera del alcance si no se aprueba |
| **d. `[workspace.lints.rust] dead_code = "deny"`** en `Cargo.toml` raíz y `[lints] workspace = true` en cada crate | Cada `cargo build` y `cargo test` | Edita 4 manifiestos | Rompe el compilado local ante cualquier función sin usar, **incluido el paso RED de TDD** (una función stub sin llamador). Más estricto que el flujo del repo. No lo recomiendo salvo que se acepte `#[cfg_attr(not(test), …)]`, que ya es un `allow` disfrazado |

Recomendación: **a + b**, y (c) como pregunta aparte.

### D. Opcional: latencia y `selftest`

- **Latencia como umbral:** fuera. Los percentiles dependen de la CPU y del modelo real. Un umbral que se afloja cada vez que falla es peor que nada. Lo útil es el `--diff` de agent-view, que está en el backlog por otro motivo. Más adelante.
- **`clawg selftest`** (serve, save, search, stop sobre un directorio temporal): vale cuando haya un segundo sistema operativo que probar de verdad. Hoy el costo es un subcomando nuevo en la superficie del binario, que hay que documentar y mantener, para algo que los tests de integración cubren en proceso. Además **no probaría lo difícil**: la descarga y carga del modelo real, el lock entre procesos y el PATH. Fuera de este cambio; si el usuario compila en macOS, vale su propio cambio.

## Descartado

- **Un test por campo y por descripción** (más de lo que ya hay). Son N aserciones a mano que se desactualizan. El snapshot lo reemplaza con un solo mecanismo.
- **Fijar el contrato desde los docs** (generar el JSON a partir de la spec o al revés). Dos fuentes de verdad y una dirección de sincronización a mantener. El código manda; los docs se comprueban contra él.
- **`#![deny(dead_code)]` en la raíz de cada crate.** Mismo problema que (d): ataca el paso RED de TDD.
- **Congelar `serverInfo.version`.** Cambia en cada bump y es parte del flujo normal de versionado.
- **Un hook de git versionado.** Los hooks no se clonan con el repo salvo con `core.hooksPath`; sería un gate que sólo existe en una máquina.
- **Comparar el JSON como string sin ordenar.** El orden de claves de `schemars` no es parte del contrato.
- **Generar los docs desde el código, con un `--check`** (la tabla de herramientas y las constantes que citan specs y README: piso de coseno, `RELEVANCE_RATIO`, banda del detector, versión de migración). Agrega un paso de generación y una segunda dirección de sincronización. Los tests ya fijan las constantes en el código (`crates/clawg-cli/src/relevance.rs:42`, `crates/clawg-core/src/memory/score.rs:407`), y las specs archivadas son fotos de su momento, no docs vivos. Lo que más se atrasa es la lista de herramientas, y la cubre el inventario de B.

## Alcance candidato

1. Test de snapshot del contrato del agente en el `mod tests` de `server.rs`, con el JSON versionado y el modo bendecir.
2. El JSON inicial, generado una vez y revisado a mano.
3. Test que compara los nombres de herramientas de `.claude/CLAUDE.md` con los registrados.
4. Quitar `age_days_from` (con sus tests), mover `id_of` a `#[cfg(test)]`, resolver `Chunk::token_count` y borrar `insert_row_with_embedding`.
5. Test que falla ante `#[allow(dead_code)]` en las fuentes (compuerta a).
6. `.claude/CLAUDE.md`, sección Testing: el comando de `clippy` (compuerta b) y cómo bendecir el contrato.
7. Arreglos de docs verificados y baratos: `AGENTS.md` (ver decisión), `clawg memories` en `.claude/CLAUDE.md`, el comentario de `AUTO_KEEP`.

### No-alcance

- Cambiar el contenido de las descripciones o de las instrucciones (eso es 22).
- Un workflow de CI (sólo la pregunta).
- `clawg selftest`.
- Umbrales de latencia.
- Comparar `agent-view` en `rag-eval --diff` (backlog aparte).
- `clawg stop` mientras carga el modelo y los otros arreglos del backlog que no son de convención o de docs.
- Corregir `clawg purge`.
- Los dos `#[allow(clippy::too_many_arguments)]`.
- Reescribir las specs archivadas.

## Decisiones abiertas

Son del usuario.

- [ ] **Dónde vive el JSON del contrato.** ¿Junto a `server.rs`, en `crates/clawg-cli/tests/` o en `docs/`? Lo que mejor va con "se revisa en el diff" es dentro del crate.
- [ ] **Cómo se bendice.** `CLAWG_BLESS=1`, un test `#[ignore]` que regenera, o un `cargo run` auxiliar. Recomendado: `CLAWG_BLESS=1` con falla igual en esa corrida.
- [ ] **¿Qué entra en el snapshot?** ¿Sólo nombre, descripción, schema e instrucciones, o también las `annotations` de las herramientas (si las hay)? Lo que decide el modelo es lo primero.
- [ ] **`AGENTS.md`.** ¿Se actualiza a mano, se reemplaza por un puntero a `.claude/CLAUDE.md` o se borra? Hoy duplica y está atrasado (siete herramientas).
- [ ] **¿También la spec `mcp` en el test de inventario?** Recomendado que no (frase en español, redacción libre).
- [ ] **`Chunk::token_count`.** ¿Se borra (toca cinco sitios de construcción y dos tests) o se deja con un lector real? "Nothing reads it yet" es la definición de especulativo.
- [ ] **La compuerta.** ¿(a) y (b) solas, o también un workflow de GitHub Actions (c)? ¿Y si se agrega Actions, se prueba macOS? install-smoke (40) necesita la misma decisión para probar Linux con el binario real: conviene tomarla una sola vez, en 40, y que (c) corra ahí.
- [ ] **¿`clippy` limpio hoy?** Hay que correrlo antes de documentarlo. Si no pasa, ¿se arregla en este cambio o se acota a `dead_code`?
- [ ] **Los arreglos de docs.** ¿Entran en este cambio o quedan en el backlog del README? Recomendado: los de `.claude/CLAUDE.md` y el comentario de `AUTO_KEEP` entran (son una línea cada uno y los cubre el test de inventario); los textos históricos del README quedan fuera.
- [ ] **Línea base de tests.** `.claude/CLAUDE.md:84` y `config.yaml:21` dicen 1864. ¿Se agrega un test de que el número esté al día? Recomendado que no: cambia en cada commit.

## Cómo se mide

Tests RED primero.

1. **Snapshot:** con el JSON inicial borrado o alterado (una palabra de una descripción, un campo de un schema), el test falla y el mensaje nombra la herramienta que cambió.
2. **Bendecir:** con `CLAWG_BLESS=1` el archivo se reescribe, la corrida falla igual, y la siguiente pasa. Sin la variable, nunca escribe.
3. **Orden estable:** dos corridas seguidas producen el mismo JSON byte a byte (no depende del orden de registro del router).
4. **Excluidos:** subir la versión del workspace no hace fallar el snapshot.
5. **Inventario:** un nombre agregado a `CLAUDE.md` que no existe, y una herramienta registrada que falta en `CLAUDE.md`, hacen fallar el test. Hoy `AGENTS.md` fallaría con cuatro faltantes si se incluyera.
6. **Compuerta (a):** un `#[allow(dead_code)]` agregado en una fuente hace fallar el test; sin ninguno pasa. Hoy falla con tres hallazgos.
7. **Código muerto:** `cargo test --workspace` termina sin el warning de `insert_row_with_embedding`, y el resto de la suite (baseline 1864, menos los tests de `age_days_from` que se van con él) sigue en verde.
8. **Si se documenta `clippy`:** el comando documentado termina con código 0 en la rama, en Windows con `--target-dir` aparte.

No toca ranking ni retrieval: `rag-eval --diff` no aplica.

## Riesgos

- **Bendecir sin mirar.** El snapshot sólo vale si el diff del JSON se revisa. Mitigación: la corrida con `CLAWG_BLESS=1` falla igual y deja el diff a la vista.
- **Ruido de dependencias.** Una actualización de `rmcp` o `schemars` cambia el schema generado y rompe el snapshot sin tocar nuestro código. Es el comportamiento buscado; el costo es revisar el diff al subir la versión.
- **Tests que leen archivos fuera del crate.** Dependen de que el repo esté entero. Si estos crates se publicaran o se probaran desde un tarball, fallarían. A verificar antes de adoptarlo.
- **Compuerta que nadie corre.** Sin CI, (b) es una frase en un documento. Por eso (a) va en la suite.
- **Borrar de más.** `age_days_from` se borra con sus tests (baja el conteo de la línea base), y `token_count` toca varios sitios. Cada uno es un commit atómico y con la suite en verde.
- **Premisas ya resueltas.** `specs/mcp` ya dice once herramientas: este cambio no la reescribe. Todo lo de docs se limita a lo verificado arriba.
- **Números de línea del backlog.** `insert_row_with_embedding` está en `worker.rs:1280`, no en `:1247`; los números de línea de docs y specs se desfasan con cada commit. No se intenta fijarlos con un test.
