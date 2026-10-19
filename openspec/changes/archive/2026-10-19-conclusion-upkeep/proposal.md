# conclusion-upkeep — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 5 | S–M | knowledge-synthesis, topic-revisions y recoverable-delete (archivados) | ver [`state.yaml`](state.yaml) |

## Problema

Una conclusión se guarda una vez y nadie la vuelve a mirar. Detalle verificado en la [exploración](exploration.md#premisas-que-se-verificaron):

1. **`synthesize` no mira las fuentes.** `CONCLUSION` lista las conclusiones vivas (`crates/clawg-core/src/memory/synthesis.rs:38-39`) sin mirar si sus fuentes siguen vivas. Una conclusión cuyas tres fuentes se borraron sale igual que una recién hecha.
2. **`derived_from` acepta lo reemplazado.** La validación sólo exige filas propias y no borradas (`synthesis.rs:199-201`, llamada en `crates/clawg-cli/src/daemon/api.rs:1023-1034`). Una conclusión nueva puede apoyarse en una versión que una revisión por clave o un veredicto ya dejó `superseded`.
3. **`purge` borra el rastro.** `DELETE FROM memory_derivation WHERE memory_id = ?1 OR source_id = ?1` (`crates/clawg-core/src/memory/secrets.rs:108`): la conclusión pierde la fuente sin rastro, y si era la única deja de ser conclusión y vuelve a pendientes como una memoria cualquiera.
4. **Nadie avisa.** El aviso de `recall_context` sólo cuenta memorias sin revisar (`synthesis.rs:166-184`, `crates/clawg-cli/src/mcp/tools.rs:394-403`).

## Decisiones

Tomadas sobre las [decisiones abiertas](exploration.md#decisiones-abiertas):

- **Estados que mandan a revisión:** una fuente borrada, `superseded` o purgada. `disputed` no: la disputa ya sale en ACTIVE CONFLICTS y no dice que la fuente sea falsa.
- **Umbral:** basta una fuente perdida. La línea dice cuántas quedan vivas de cuántas, y la lista lleva tope.
- **B:** `derived_from` rechaza con 400 una fuente `superseded` y nombra su reemplazante.
- **C:** `purge` deja los vínculos en los que la fila purgada es fuente (sólo un id, nunca texto) y sigue borrando los de la fila purgada como conclusión. El id no se reusa (`AUTOINCREMENT`, `crates/clawg-core/migrations/V1__init.sql:5`).
- **Aviso:** línea propia en `recall_context` y en el hook `session-start`, desde una conclusión, contando sólo las pérdidas posteriores a la última revisión del alcance, para que el aviso se apague al llamar `synthesize` aunque la conclusión siga en la lista. Alternativas en el [diseño](design.md#d3-el-aviso).

## Alcance

1. Regla pura `needs_review` y su tabla de casos.
2. `POST /v1/synthesis` suma `to_review` (hasta 10, la más nueva primero) y `to_review_total`; `synthesize` las muestra con sus fuentes perdidas y qué hacer con ellas.
3. `POST /v1/context` suma `conclusions_to_review`; `recall_context` y `session-start` agregan la línea.
4. `derived_from` rechaza fuentes `superseded`.
5. `purge` conserva los vínculos de la fila purgada como fuente.
6. `.claude/CLAUDE.md` y las specs `synthesis` y `secrets`.

## Fuera de alcance

- Columnas de confianza (D de la exploración).
- Borrar o reescribir conclusiones solas.
- Cambios de ranking o de lo que `recall_context` recupera.
- `disputed` como estado de revisión.

## Datos guardados

- **Esquema:** sin cambios. Ninguna migración.
- **Purge:** deja de borrar filas de `memory_derivation` cuyo `source_id` es la fila purgada. Un binario anterior lee esos vínculos como cualquier otro: `derived_from` nombra un id que ya no existe, que es lo que ya veía de una fuente borrada.
- **Rollback:** volver al binario anterior no necesita nada. Los vínculos a ids purgados quedan; se borran a mano con `DELETE FROM memory_derivation WHERE source_id NOT IN (SELECT id FROM memory);`.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)): los seis de la [exploración](exploration.md#cómo-se-mide), más purgar una fuente que deja la conclusión en revisión y el aviso que se apaga al revisar. `rag-eval` no aplica: no cambia el ranking ni el prompt que arma `build_memory_prompt`; la línea nueva va fuera de él, como la de síntesis.
