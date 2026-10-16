# quality-guards — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S | — (conviene que cierre antes de 22, 23, 25 y 27, que tocan la superficie MCP) | ver [`state.yaml`](state.yaml) |

## Problema

Lo que el modelo ve del servidor (nombres, descripciones y schemas de las herramientas, y las instrucciones) puede cambiar en cualquier commit sin que falle un test, los docs que enumeran esa superficie se atrasan, y la regla de no dejar código muerto no la hace cumplir nadie. Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código); lo que sigue está verificado otra vez contra `main` (`638812b`):

1. **La superficie no está fijada.** Las instrucciones son la constante `INSTRUCTIONS` (`crates/clawg-cli/src/mcp/server.rs:108-119`) y hay doce herramientas (`#[tool(` en `server.rs:810` a `:997`). Los tests fijan los nombres y el conteo (`every_tool_is_registered_under_its_wire_name`, `server.rs:1537-1562`; `the_viewer_is_not_exposed_to_the_model`, `server.rs:1569-1604`), las cinco propiedades de `list_memories` y algunas palabras de las instrucciones y del schema de `save_memory` (`server.rs:1510-1534`, `:1610-1642`). Nadie fija el resto: se puede reescribir la descripción de `delete_memory`, quitar un campo de `search_memory` o acortar las instrucciones, y `cargo test` pasa. Los schemas los genera `schemars` (`rmcp` 3.5.1, `schemars` 1.2.1 en `Cargo.lock`), así que subir una de esas dependencias también puede cambiarlos sin tocar una línea nuestra.
2. **Los docs se atrasan.** `AGENTS.md` es una copia vieja de `.claude/CLAUDE.md`: lista siete herramientas de doce (`AGENTS.md:39`), le falta la mitad de la API HTTP (`AGENTS.md:37`) y siete comandos, y dice «1304 tests» (`AGENTS.md:71`). `README.md:412` manda a leer `AGENTS.md`. El comentario de `AUTO_KEEP` nombra dos snapshots automáticos (`crates/clawg-core/src/memory/backup.rs:227`) cuando `is_automatic` cubre tres (`backup.rs:143-145`, incluye `pre-reembed`). `openspec/specs/storage/spec.md:225` cita `AUTO_KEEP` en `backup.rs:226` (está en `:230`). `README.md:110` y `:143` dan `clawg 0.2.0` como ejemplo de versión (la del workspace es 0.5.1).
3. **Hay código muerto escondido.** Tres `#[allow(dead_code)]` contra la convención «no `#[allow]` to hide dead code» (`.claude/CLAUDE.md`, Code conventions): `TimestampMs::age_days_from` (`crates/clawg-core/src/memory/clock.rs:99`), `RefMap::id_of` (`crates/clawg-eval/src/bin/rag-eval/golden.rs:167`) y el campo `Chunk::token_count` (`crates/clawg-core/src/memory/chunker/mod.rs:97`), que sólo leen tests. Y un helper de test sin llamador, `insert_row_with_embedding` (`crates/clawg-core/src/memory/worker.rs:1280`), que deja un warning en cada `cargo test`.
4. **No hay compuerta.** No existe `.github/`, ni `clippy.toml`, ni `[lints]` en ningún manifiesto. Sin `-D warnings`, un `dead_code` no rompe nada.

## Decisiones

- **`AGENTS.md` pasa a ser un puntero** a `.claude/CLAUDE.md` (tarjeta en el hilo, opción recomendada; mientras Marcos no elija otra, se sigue con ésta). La causa del atraso es tener dos copias.
- **Snapshot del contrato** (enfoque A): un JSON versionado junto a `server.rs` con cada herramienta tal como sale en `tools/list` y el texto de las instrucciones, sin el nombre ni la versión del servidor. Se bendice con `CLAWG_BLESS=1`, que reescribe el archivo y falla igual. Detalle en el [diseño](design.md).
- **Inventario sólo contra `.claude/CLAUDE.md`** (enfoque B): su línea `MCP tools (...)` tiene un formato estable. La spec `mcp` no entra: los nombres van dentro de una oración en español y la mantiene el archivo de cada cambio.
- **`Chunk::token_count` se borra.** «Nothing reads it yet» es la definición de especulativo; los tests que lo leen ya fijan el texto exacto de cada pedazo.
- **Compuertas a y b** de la exploración: un test que falla ante un `allow` de `dead_code` o `unused` en las fuentes, y el comando de `clippy` en la sección Testing de `.claude/CLAUDE.md`. Un workflow de CI se decide una sola vez en install-smoke (40).
- **`clippy` acotado a los warnings del compilador.** Medido en `main`: `clippy -D warnings` no pasa, con 29 hallazgos propios de `clippy` y uno de `rustc` ([diseño, D7](design.md#d7-clippy-documentado)). El comando documentado es `-A clippy::all -D warnings`, que pasa después de este cambio; los de `clippy` van al backlog.

## Alcance

1. Test de snapshot del contrato y el JSON inicial, revisado a mano.
2. Test de inventario: las herramientas de `.claude/CLAUDE.md` son las registradas.
3. Quitar el código muerto: `age_days_from` con sus tests, `id_of` bajo `#[cfg(test)]`, el campo `token_count` y `insert_row_with_embedding`.
4. Test de convención: ningún `allow` de `dead_code` o `unused` en `crates/*/src` ni `crates/*/tests`.
5. `.claude/CLAUDE.md`, Testing: el comando de `clippy` y cómo bendecir el contrato.
6. Docs: `AGENTS.md` como puntero y `README.md:412` apuntando a `.claude/CLAUDE.md`; el comentario de `AUTO_KEEP`; la referencia de `specs/storage`; `README.md:110` y `:143` sin número de versión.

## Fuera de alcance

- Cambiar el texto de las descripciones o de las instrucciones (memory-protocol, 22).
- Un workflow de GitHub Actions y probar macOS (install-smoke, 40).
- `clawg selftest` y umbrales de latencia (exploración, enfoque D).
- Los dos `#[allow(clippy::too_many_arguments)]` (`bootstrap.rs:463`, `prompt_builder.rs:1296`): no esconden código muerto.
- El conteo «once herramientas» de `openspec/specs/mcp/spec.md:46`, `:59`, `:65`: lo pone al día el archivo de recoverable-delete, que agregó `restore_memory`.
- Las specs archivadas y el texto histórico de `openspec/README.md`.

## Datos guardados

No toca el esquema, ni datos guardados, ni el contrato del embedder. Todo el cambio es tests, docs y código sin llamador. Rollback: revertir los commits; no hay nada que migrar.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- el snapshot falla si cambia una palabra de una descripción, un campo de un schema o las instrucciones, y nombra la herramienta;
- con `CLAWG_BLESS=1` reescribe el archivo y falla; la corrida siguiente pasa;
- el inventario falla si `.claude/CLAUDE.md` nombra una herramienta que no existe o le falta una registrada;
- el test de convención falla hoy con tres hallazgos y pasa después;
- `cargo test --workspace` termina sin warnings de compilación.

No toca ranking ni retrieval: `rag-eval` no aplica. La suite baja en los tests que se van con `age_days_from` y sube en los nuevos; `verify.md` da el conteo contra la base (1902 en Windows).
