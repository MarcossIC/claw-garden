# mcp-protocol-upgrade — Verificación

**Estado:** completo. En Linux el cambio no agrega fallas (siguen las 10 de entorno de la base); en Windows pasa todo el suite, y Claude Code real conecta al perfil dev, se nombra desde el cliente y ve las memorias de antes ([Windows](#windows-2026-10-13), [Claude Code real](#claude-code-real-perfil-dev-2026-10-13)).

## Tests

### Linux

`cargo test --workspace --no-fail-fast`.

| Corrida | Pasan | Fallan |
|---|---|---|
| Base (`origin/main`, 1127b50) | 1.675 | 10 |
| Con el cambio (08dc9ba) | 1.685 | 10 |
| Base después de visibility-policy (`origin/main`, c01a0af) | 1.696 | 10 |
| Con el cambio sobre ese main (53db597) | 1.706 | 10 |

Son 10 tests más y ninguna falla nueva. Las 10 fallas son las mismas en las dos corridas y vienen del entorno, no del código:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- 3 de `diff::tests` en `rag-eval`: leen resultados de `docs/eval/results/`, que está en `.gitignore` y no existe en un clon limpio.

De dónde salen los 10: `server.rs` cambia un test de identidad por cuatro puros y suma tres por el cable (+6), y `handshake.rs` pasa de 10 a 14 tests.

### Los tests nuevos fallan sin el cambio

- **R1 y R3:** en `main`, la sonda con `_meta` recibe `-32601` del guardia, y una request sin `initialize` no la acepta rmcp 1.3.
- **R4 y R5:** con rmcp 3.5.1 y la lectura de identidad de `main` (`peer_info()`), los tres tests por el cable fallan: la sesión sin `initialize` guarda y busca bajo `"mcp"`.
- **El guardia (D1):** sin `PreInitGuard`, `a_bare_probe_and_its_cancellation_leave_the_legacy_handshake_working` falla, porque la cancelación de la sonda termina el puente. Con el guardia que se apagaba al pasar la primera request, `a_request_refused_for_missing_meta_does_not_start_the_session` falla por la misma razón.

### rag-eval

No aplica. El cambio no toca `clawg-core`, el ranking ni el golden set.

## Windows (2026-10-13)

`cargo test --workspace --no-fail-fast --target-dir target-test` sobre `main` (46c6245), que ya tiene el cambio (#14) y lo que entró después (`synthesize`, la octava herramienta).

| Pasan | Fallan |
|---|---|
| 1.655 | 0 |

Es la línea de base de `CLAUDE.md`. Los tests del cambio (`handshake.rs`, los de identidad de `server.rs` y `initialize_is_answered_while_the_daemon_is_still_starting` de `mod.rs`) están entre los que pasan.

## Claude Code real, perfil dev (2026-10-13)

Binario debug de `main` (`target/debug/clawg.exe`, compilado con rmcp 3.5.1), Claude Code 2.1.296.

**Con el registro de siempre** (`clawg-dev`, `--agent claude-code`): el MCP conecta sin `CONNECT_TIMEOUT` y sus herramientas quedan disponibles en la sesión. `memory_status` dice `agent: claude-code (from --agent)` y `list_memories` muestra la memoria #2, guardada antes del cambio.

**Sin `--agent`**, que es lo que prueba R4 y R5: con ese registro el nombre lo fija el flag, así que no ejercita la lectura de `clientInfo`. Se corrió un Claude Code aparte con el mismo binario y perfil:

```
claude -p ... --mcp-config '{"mcpServers":{"probe":{"type":"stdio","command":".../target/debug/clawg.exe","args":["mcp","--profile","dev"]}}}' --strict-mcp-config
```

- `memory_status`: `agent: claude-code (reported by the client; pin it with --agent)`. El nombre sale del cliente y no es `mcp`.
- `search_memory` ("¿Cómo se llama el bar de la esquina de Marco?") encuentra la #2 con score 0,64: el cliente sin `--agent` ve lo que se guardó antes del cambio.
- `list_memories` muestra la misma #2.

No se guardó nada en esta prueba: `memory_status` informa el `agent_id` con el que se guardaría, que es el mismo bajo el que están las filas anteriores. No se determinó si Claude Code 2.1.296 usó el ciclo legacy o el 2026-07-28; la spec acepta los dos (R2 y R3), y el nombre resultó `claude-code` igual.
