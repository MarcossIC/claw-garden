# save-bounds — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — (después de 53; antes de 44) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`29a5739`).

1. **`content` no tiene tope.** El daemon sólo rechaza el vacío (`save_memory`, `crates/clawg-cli/src/daemon/api.rs:1069-1071`), y el puente igual (`save_memory_impl`, `crates/clawg-cli/src/mcp/server.rs:712-714`). Lo único que acota el cuerpo es el defecto de 2 MB del extractor `Json` de axum: no hay `DefaultBodyLimit` en el repo.
2. **El costo de un save largo crece con el cuadrado de su tamaño.** Cada pedazo reescribe la fila del outbox con todo el progreso anterior más el suyo (`note_for`, `crates/clawg-core/src/memory/worker.rs:629-646`; `apply_note`, `crates/clawg-core/src/memory/outbox.rs:117-122`), y cada `UPDATE` reescribe también `content`, que vive en la misma fila (`crates/clawg-core/migrations/V14__ingest_outbox.sql:11-24`). Todo eso corre en la única conexión de escritura y en el único worker de ingest, con un KNN completo del detector por pedazo (`worker.rs:905-907`).

Medido con el embedder falso y el tokenizador de prueba, sumando con un trigger los bytes de `progress` de cada `UPDATE` (ver [verify](verify.md#medición-del-progreso)):

| `content` | Pedazos | Bytes de `progress` escritos | Tiempo del save |
|---|---|---|---|
| 8 KiB | 29 | 0,13 MB | 0,05 s |
| 16 KiB | 57 | 0,52 MB | 0,09 s |
| 32 KiB | 114 | 2,1 MB | 0,19 s |
| 64 KiB | 228 | 8,3 MB | 0,52 s |
| 128 KiB | 455 | 33 MB | 1,6 s |
| 256 KiB | 910 | 133 MB | 5,3 s |

Cada duplicación del texto cuadruplica los bytes y algo más que triplica el tiempo. Extendida hasta los 2 MB que deja pasar axum (unos 7 300 pedazos) la curva da del orden de 8 GB de progreso.

## Objetivo que sirve

1: guardar es eficiente y su costo se conoce antes de aceptar el save. Va antes de 44 (history-import), que trae miles de notas por este camino.

## Decisiones

Las decisiones abiertas de la exploración se cierran así; el porqué está en el [diseño](design.md).

- **Unidad del tope:** bytes de `content` tal como llega (`str::len`), sin tokenizador.
- **Valor:** 64 KiB (65 536 bytes). Deja pasar la nota más larga que midió 44 (45 372 caracteres) y acota el peor caso a unos 228 pedazos, 8 MB de progreso y medio segundo de hilo con el embedder falso.
- **Progreso:** sin cambio de formato ni migración (ni B ni C). Con el tope, el cuadrático queda acotado a lo medido.
- **`DefaultBodyLimit`:** no. El defecto de 2 MB de axum ya acota lo que se deserializa.
- **Dónde:** el daemon rechaza; el puente no comprueba nada propio, muestra la frase del daemon.
- **44:** decide en su propio cambio qué hace con una nota sobre el tope; ninguna de las medidas lo supera.

## Alcance

1. `api.rs`: `MAX_CONTENT_BYTES` y `check_content` (pura), que reemplaza la comprobación de vacío: `400` antes del estado del embedder y del outbox, con el tamaño recibido, el tope y qué hacer.
2. `server.rs`: la descripción de `content` en `save_memory` nombra el tope; `agent_contract.json` bendecido.
3. `protocol.md`: «One fact per call» dice el tope y que un texto más largo se parte.
4. Docs: `.claude/CLAUDE.md`, y al archivar `openspec/specs/ingest`.

## Fuera de alcance

- El formato de `progress`, una tabla por pedazo o una migración.
- Embeber por lotes o juntar los KNN del detector (D y E de la exploración); el costo de cada KNN es de knn-prefilter (37).
- Partir un save largo en el daemon o truncarlo.
- El tope y el plazo de las conexiones (sigue en el backlog del `README.md`).
- Ranking: no se corre `rag-eval`.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), sin modelo real, red ni perfil:

- un save de 65 537 bytes responde `400` con el tope en el mensaje y el outbox queda vacío; hoy se acepta;
- uno de 65 536 bytes se acepta y se guarda;
- `check_content` cuenta bytes, no caracteres: 32 769 `ñ` (65 538 bytes) se rechazan;
- el puente devuelve `not saved: content is …` con el tope.

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni el contrato de embedding. Sólo rechaza saves nuevos más grandes que el tope; lo ya guardado y una fila del outbox de cualquier tamaño, escrita antes, se siguen leyendo y terminando como hoy (el tope está en el handler, no en el worker). Volver atrás es volver al binario anterior.
