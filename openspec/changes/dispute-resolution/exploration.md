# dispute-resolution — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 5 | M | eval-golden-expansion (los pares de contradicción ya existen) | exploración (cerrada) |

**Tesis:** las contradicciones **se detectan** y nunca **se resuelven**. Una memoria disputada queda así para siempre, o hasta que alguien la borra.

## Problema (verificado)

### Detección (activa)

- **Cuándo corre:** al ingerir (`crates/clawg-core/src/memory/worker.rs:363`).
- **Qué compara:** los 5 vecinos más cercanos, sólo de `memory`, del mismo agente y proyecto. Ambas filas tienen que cargar afirmaciones.
- **Criterio:** distancia L2 en [0,05; 0,35), o sea coseno de ~0,94 a 0,999 (`crates/clawg-core/src/memory/contradiction.rs:48-54, 118-149`).
- **Qué escribe:**
  - la fila **nueva** se inserta con `status='disputed'`, `disputed_against` y `last_disputed_at` (`crates/clawg-core/src/memory/store_insert.rs:313-316`);
  - la vieja sigue `active`.
- **Problemas:**
  - Es proximidad vectorial pura. No distingue una contradicción de un hecho complementario o de una paráfrasis.
  - **Hipótesis sin verificar:** los chunks hermanos de un mismo save largo podrían disputarse entre sí, porque se insertan de a uno (`worker.rs:356-378`).

### Resolución (no existe)

- `resolver.rs` no tiene llamador en producción.
- Ni siquiera ese resolver escribía `superseded`: `apply_resolution` deja a los perdedores en `disputed` (`crates/clawg-core/src/memory/resolver.rs:39, 163-236`).
- Ningún código escribe `status='superseded'` ni `superseded_by`. Sólo se leen o se filtran.
- La única salida de `disputed` es `delete_memory`.

### Mitigaciones recientes

- `46c518c`: una fila `disputed` puntúa igual que una activa.
- `aa52e3b`: el bloque de conflictos sólo muestra pares que tocan memorias mostradas.
- El bloque le pide al agente que consulte al usuario (`crates/clawg-core/src/memory/prompt_builder.rs:315-316`).

### Disputas huérfanas

- Si se borra la fila **vieja**, la nueva queda `disputed` para siempre.
- El bloque de conflictos desaparece, porque exige `m_old.deleted_at IS NULL`.

### Workspace

- Sus queries no filtran por estado, así que una fila `disputed` puede aparecer en `workspace.md`.

### Lo que no hay que asumir

- **La detección no está parada.** Corre en cada save explícito.
- **Ningún código marcó nunca `superseded`**, ni siquiera el resolver muerto.

## Enfoques

| Opción | A favor | En contra |
|---|---|---|
| a. **Agente en el loop:** herramienta MCP `resolve_conflict(winner, loser)`. El perdedor pasa a `superseded` con `superseded_by` | El agente ya es un LLM: no hace falta clave ni proveedor, y es local-first. Encaja con lo que hoy ya pide el bloque de conflictos | Depende de que el agente lo use. Sin agente activo, las disputas quedan esperando |
| b. **Revivir el resolver LLM** en background | Automático | Necesita proveedor, clave y disparador, que hoy no hay. Choca con local-first, salvo que se use Ollama |
| c. **Heurística automática** (por ejemplo, la más nueva gana después de N días) | Sin LLM | Arriesgado: puede tapar el hecho correcto |

Aparte de cómo se resuelva, falta:

- arreglar la disputa huérfana al borrar (la fila sobreviviente vuelve a `active`);
- filtrar las `disputed` del workspace.

## Decisiones abiertas

- [x] a: herramienta MCP `resolve_conflict`, con "una gana" y "valen las dos". b queda para knowledge-synthesis; c se descarta. Ver `proposal.md`.
- [x] La precisión del detector va en otro cambio. Éste sólo excluye a los pedazos hermanos de un mismo save.
- [x] `superseded` sale del ranking, del contexto y del workspace, y sigue en `list_memories`. `disputed` sigue en el workspace (ver `proposal.md`).

## Cómo se mide

- **Precisión y recall del detector** sobre los 28 pares de contradicción del golden set, antes y después.
- **Tests de transiciones:**
  - `disputed` → `active` + `superseded`;
  - borrar la vieja → la nueva queda `active`;
  - las filas `disputed` no aparecen en el workspace.
- **Test de la hipótesis de chunks hermanos**, para confirmarla o descartarla.
