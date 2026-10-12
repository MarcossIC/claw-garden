# visibility-policy — Verificación

**Estado:** parcial. Partes A (identidad) y B (scope de lo que modifica). Los tests pasan en Linux y el binario debug respondió bien en una prueba por stdio con un perfil `dev` temporal. Falta correr en la máquina de Marcos lo que necesita Windows y su perfil `dev` (abajo, **Pendiente**). No se archiva hasta entonces.

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux.

| Corrida | Pasan | Fallan |
|---|---|---|
| Base (`origin/main`, 1127b50) | 1.675 | 10 |
| Con el cambio (7f04101) | 1.696 | 10 |

Son 21 tests nuevos y ninguna falla nueva. Las 10 fallas son las mismas en las dos corridas y vienen del entorno, como en los cambios anteriores:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- 3 de `diff::tests` en `rag-eval`: leen resultados de `docs/eval/results/`, que está en `.gitignore`.

Cada commit del cambio se probó por separado, con el mismo comando: todos compilan y ninguno agrega fallas (1.678, 1.681, 1.684, 1.692, 1.696 y 1.696 pasan; 10 fallan en todos, las mismas).

Los tests nuevos fallaban antes de su código: los de firma nueva (`owner`, `--agent`, `workspace_segment`, el agente en los `*_impl`) no compilaban, y los HTTP de la tarea 2 se corrieron en rojo (404 esperado, la fila se borraba o cambiaba).

### Windows

**Pendiente.** `cargo test --workspace --target-dir <otro-dir>` en la máquina de Marcos. Lo que más puede diferir: `workspace_segment` (`\` y `:` se codifican; la ruta de `claude-code` no cambia) y los tests de `workspace_path` que comparan rutas.

## Prueba con el binario

Binario debug de 7f04101, con `HOME` apuntando a un directorio temporal (perfil `dev` vacío, sin tocar `~/.clawg*` de nadie). Un script habló MCP por stdio con `clawg mcp --profile dev --agent claude-code`, declarándose como `clientInfo.name = "Claude Code"`:

- el puente arrancó el daemon solo;
- `memory_status` terminó con `agent: claude-code (from --agent)`: el flag ganó sobre el nombre del cliente;
- `list_memories` respondió `no memories stored`;
- `clawg stop --profile dev` paró el daemon.

## rag-eval

No aplica. El cambio no toca la búsqueda: ni `search.rs` (salvo un argumento `None` en dos tests) ni el predicado de search/recall ni el ranking. `rag-eval` llama a la búsqueda directamente, sin las operaciones que ahora tienen scope.

## Pendiente (máquina del usuario)

1. **Suite en Windows:** `cargo test --workspace --target-dir <otro-dir>`. Se espera la base de Windows (`CLAUDE.md`: 5 fallas en `project_root.rs`) más los 21 tests nuevos en verde.
2. **Perfil dev con Claude Code:**
   - Ver con qué nombre guarda hoy Claude Code: `clawg memories stats --profile dev` (agrupa por agente) o la herramienta `memory_status`, que ahora dice el agente y su origen (`reported by the client`).
   - Registrar el puente con ese mismo nombre: `claude mcp add clawg-dev -- <ruta>/target/debug/clawg mcp --profile dev --agent <nombre>`.
   - En una sesión nueva: `memory_status` dice `agent: <nombre> (from --agent)`, y `search_memory` y `list_memories` siguen viendo lo guardado antes.
   - Con otro cliente o nombre (por ejemplo `--agent otro`), `delete_memory` sobre un id del primero responde error y la memoria sigue en `clawg memories list`.

## Solapamiento con otros cambios en curso

- **mcp-protocol-upgrade (#14, en draft):** acordado con ese thread. Este cambio entra primero; después ese PR rebasa y sólo cambia de dónde sale el nombre del cliente dentro de `agent_for` (`RequestContext::client_info()` en rmcp 3.5.1). `resolve_agent` y la precedencia quedan como están.
- **secret-hygiene:** sigue antes de la parte C (compartir), como pide el mapa. Este cambio no comparte nada.
- **dispute-resolution:** `resolve_conflict` ahora exige que las dos filas sean del agente. El detector ya compara sólo dentro del mismo agente, así que una disputa real nunca cruza agentes.
- **Migraciones:** ninguna.
