# project-identity — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 | M | — | exploración (borrador) |

**Tesis:** el mismo repositorio termina con varios `project_id` según cómo se lo nombre. Las memorias quedan partidas entre ellos, sin aviso.

## Problema (verificado)

**Cómo se calcula el id**

- `derive_project_id` arma `{basename en minúsculas}-{4 hex}`, donde los 4 hex son SipHash-1-3, con clave cero, sobre los **bytes crudos** de la ruta (`crates/clawg-core/src/memory/identity.rs:11-24`).

**De dónde sale la ruta**

- Del cwd del proceso puente, leído una sola vez al arrancar (`crates/clawg-cli/src/mcp/server.rs:165`).
- Por llamada, `project_path` lo sobreescribe y también se hashea crudo (`crates/clawg-cli/src/mcp/tools.rs:153-166`).

**Particiones falsas**, cada una con un id distinto:

- caja (`C:\X` contra `c:\x`);
- separadores (`/` contra `\`);
- separador final;
- ruta relativa;
- subdirectorio;
- worktree de git;
- segundo clon.

**Si falla `current_dir()`**, el resultado es `None`. Las búsquedas salen sin scope y los saves quedan con `project_id` NULL, visibles en todos lados.

**El store no guarda la ruta**, sólo el id (`crates/clawg-core/migrations/V1__init.sql`). Los ids existentes **no se pueden revertir** a una ruta.

### Lo que no hay que asumir

- **La colisión no es el problema.** Para chocar hacen falta el mismo basename y los mismos 16 bits. El problema es la partición falsa.
- **`_meta.project_root_canonical` no es estándar** y ningún cliente lo va a mandar.
- **La spec MCP 2026-07-28 deprecó Roots.** En su lugar recomienda "tool parameters o server configuration", que es justo lo que clawg ya hace (cwd más `project_path`). El arreglo va en cómo se **normaliza** esa ruta, no en el canal por el que llega.

## Alcance candidato

1. **Una función pura `resolve_project_root(path) -> CanonicalRoot`:**
   - canonicaliza, sin el prefijo `\\?\` en Windows;
   - sube hasta la raíz git (un `.git` que puede ser directorio o, en un worktree, archivo);
   - unifica separadores;
   - pasa a minúsculas en Windows.
2. Aplicarla tanto al cwd como a `project_path`.
3. **Tabla `project`** (`id`, `root_path`, `created_at`). Guarda la ruta canónica desde ahora, para poder volver a derivar el id si cambia el algoritmo.
4. **Re-key perezoso:** al arrancar el puente, calcular el id legacy de la ruta tal cual llegó y el id nuevo. Si hay filas con el legacy, pasarlas al nuevo en una transacción.

## Decisiones abiertas

- [ ] **Worktrees:** ¿comparten memorias con el repo principal (git common dir) o cada uno es su propio proyecto?
- [ ] **Directorios sin git:** ¿cwd canonicalizado como raíz, o "sin proyecto"?
- [ ] **Formato del id:** ¿se mantiene `basename-hash4` o se amplía el hash? Ampliarlo cambia todos los ids.
- [ ] **Memorias guardadas desde subdirectorios:** su id legacy sólo se recupera si el puente vuelve a arrancar en ese subdirectorio. ¿Alcanza con eso, o hace falta un comando `clawg projects merge <id> <id>`?
- [ ] **Rollback:** ¿la tabla de re-key guarda el mapeo legacy → nuevo para poder revertirlo?

## Cómo se mide

- **Tests de la función pura**, sin filesystem real donde se pueda: caja, separadores, separador final, relativa, subdirectorio, worktree, sin git.
- **Test de integración del re-key:** filas con el id legacy quedan visibles bajo el id nuevo, y una segunda corrida no hace nada (idempotencia).
- **Ninguna fila queda con `project_id` NULL** cuando falla `current_dir()`, o queda documentado que sigue siendo así.
