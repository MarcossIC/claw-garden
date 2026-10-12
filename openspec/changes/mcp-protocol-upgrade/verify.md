# mcp-protocol-upgrade — Verificación

**Estado:** parcial. Los tests pasan en Linux. Faltan el suite en Windows y la prueba con Claude Code real, las dos en la máquina de Marcos (ver [Pendiente](#pendiente-máquina-del-usuario)).

## Tests

### Linux

`cargo test --workspace --no-fail-fast`.

| Corrida | Pasan | Fallan |
|---|---|---|
| Base (`origin/main`, 1127b50) | 1.675 | 10 |
| Con el cambio (533523d) | 1.683 | 10 |

Son 8 tests más y ninguna falla nueva. Las 10 fallas son las mismas en las dos corridas y vienen del entorno, no del código:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- 3 de `diff::tests` en `rag-eval`: leen resultados de `docs/eval/results/`, que está en `.gitignore` y no existe en un clon limpio.

De dónde salen los 8: `server.rs` cambia un test de identidad por cuatro puros y suma tres por el cable (+6), y `handshake.rs` pasa de 10 a 12 tests.

### Los tests nuevos fallan sin el cambio

- **R1 y R3:** en `main`, la sonda con `_meta` recibe `-32601` del guardia, y una request sin `initialize` no la acepta rmcp 1.3.
- **R4 y R5:** con rmcp 3.5.1 y la lectura de identidad de `main` (`peer_info()`), los tres tests por el cable fallan: la sesión sin `initialize` guarda y busca bajo `"mcp"`.
- **El guardia (D1):** sin `PreInitGuard`, `a_bare_probe_and_its_cancellation_leave_the_legacy_handshake_working` falla. La cancelación de la sonda termina el puente.

### rag-eval

No aplica. El cambio no toca `clawg-core`, el ranking ni el golden set.

## Pendiente (máquina del usuario)

1. **Windows:** `cargo test --workspace --target-dir <otro-dir>`. Se espera la línea de base de `CLAUDE.md` (5 fallas en `project_root.rs`) más los tests nuevos en verde.
2. **Claude Code real, perfil dev.** Con el binario debug de esta rama registrado como el MCP `clawg-dev`:
   - el MCP conecta sin `CONNECT_TIMEOUT` y `/mcp` lista las siete herramientas;
   - una búsqueda encuentra una memoria guardada **antes** del cambio. Es la prueba de que Claude Code se nombra igual en el ciclo de vida nuevo que en `initialize`;
   - `clawg memories` (o `list_memories`) muestra lo que se guarde en esta prueba con el mismo `agent_id` que las filas anteriores, y no `mcp`.

   Si Claude Code se queda en el ciclo legacy, también vale: lo cubre R2. Si pasa al nuevo sin `clientInfo`, las filas nuevas van a quedar bajo `mcp`. En ese caso no se mergea: hace falta antes `--agent`, de visibility-policy.
