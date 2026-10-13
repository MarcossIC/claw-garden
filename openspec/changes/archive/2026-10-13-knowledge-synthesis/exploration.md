# knowledge-synthesis — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 5 | decisión de producto (después, uno o dos cambios) | — | cerrada: ver la [propuesta](proposal.md) |

**Tesis:** el objetivo 5, extraer conclusiones con el tiempo, no tiene hoy ningún camino vivo. El código que lo haría está compilado, pero le falta todo lo de alrededor: fuente de datos, proveedor, clave y disparador. Antes de escribir una spec hay que decidir si se revive, se reemplaza o se borra.

## Estado actual (verificado)

### Módulos sin llamador en producción

- `extractor.rs`, `miner.rs`, `resolver.rs`, `episodic_writer.rs` y `providers/{anthropic,ollama,openrouter}.rs`.
- Son `pub mod` sin feature gate (`crates/clawg-core/src/lib.rs:7-14`, `crates/clawg-core/src/memory/mod.rs:58-68`). Sólo los usan los tests.

### Lo que sí corre en el daemon

- `serve` lanza sólo tres cosas (`crates/clawg-cli/src/daemon/serve.rs:85-95`):
  - el bootstrap del embedder;
  - el worker de ingesta;
  - el de mantenimiento.

### Lo que falta para revivir el pipeline

- **`5678ded` borró** la configuración, las credenciales, oauth y el runtime de agente.
- **Fuente de claves:** no hay.
- **Transcripts de sesión:** no hay.
- **Disparador:** no hay.
- **Ingesta:** `POST /v1/memories` siempre manda `Role::User` con `MemorySource::Explicit` (`crates/clawg-cli/src/daemon/api.rs:440-454`).

### Tablas `episodic_trajectory` y `procedural_skill`

- **Sin escritor en producción.**
- **Se consultan igual en cada búsqueda:** KNN de 5 + 5 (`crates/clawg-core/src/memory/search.rs:32-39`).
- **Costo real:** una sentencia KNN vacía más un `COUNT` por tabla, serializadas en el hilo de la base. Es chico, pero es código muerto en el camino caliente.
- **Workspace:** la sección "skills" está siempre vacía.

### Golden set

- No tiene ninguna fila episodic ni procedural. El ranking de esas tablas no está validado.

### Convenciones del proyecto

- Hay `#[allow(dead_code)]` en el chunker, y la convención del proyecto los prohíbe.

### Decisión abierta previa

- La memoria del proyecto ya registra como abierta la decisión sobre el futuro del pipeline LLM dormido.

## Opciones

| Opción | A favor | En contra |
|---|---|---|
| a. **Borrar** el pipeline dormido, las tablas episodic/procedural y sus ramas KNN. La síntesis se construye después sobre `memory` | Menos código, menos superficie, el camino caliente queda limpio. Se cumple la convención | Se pierde trabajo previo (sigue en el historial de git) |
| b. **Revivir con hooks de Claude Code** (por ejemplo, SessionEnd o PreCompact entregan la ruta del transcript) y un proveedor (Ollama local o clave de API) | Reusa extractor y miner, que ya tienen tests | Clave o modelo local, más costo por sesión. Los transcripts traen datos sensibles. Depende de la API de hooks del cliente |
| c. **Síntesis dirigida por el agente:** una herramienta o prompt MCP que, cada tanto, le pide al propio agente que escriba conclusiones a partir de lo recuperado | Sin clave ni proveedor; local-first | Depende de que el agente coopere. La calidad varía |
| d. **Status quo** | Sin trabajo | Mantiene código muerto, KNN vacíos y violaciones de convención |

## Decisiones abiertas

Cerradas en la [propuesta](proposal.md): limpieza + opción c, y las tablas episodic/procedural se quedan porque la app de escritorio sí las escribía.

- [ ] **¿Qué significa "conclusión" en el producto?** Por ejemplo: preferencias del usuario, convenciones del proyecto, errores recurrentes, lineamientos.
- [ ] **¿a, b o c?** El resultado probable son dos cambios:
  - **limpieza:** borrar lo muerto o ponerlo detrás de un feature;
  - **síntesis:** el camino elegido.

## Cómo se mide

Depende de la opción elegida. Para la limpieza:

- `cargo test --workspace` en verde, con el conteo nuevo justificado;
- `rag-eval` con métricas idénticas, porque las tablas están vacías.
