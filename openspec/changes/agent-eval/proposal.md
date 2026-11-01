# agent-eval — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (1, 5) | L por fases; esta propuesta cubre el arnés, que no gasta (M) | — | ver [`state.yaml`](state.yaml) |

## Problema

`rag-eval` mide qué se recupera y `--agent-view` qué texto le llega al agente, pero nada mide qué hace una sesión de un agente con eso ([exploración](exploration.md), «Tesis»). Cada cambio que tocó lo que ve el agente se verificó con una sesión manual de Claude Code, que no se repite igual. Antes de gastar una sola sesión paga hace falta el instrumento: leer tareas, armar cada brazo, leer el stream del cliente y puntuarlo sin juez. Hoy no existe ninguna de esas piezas en `crates/clawg-eval` (`Cargo.toml` declara cinco binarios y ninguno lanza ni lee un cliente).

## Objetivo

Medición (1, 5): el arnés de la fase 0 del [roadmap](exploration.md#roadmap) sin el fixture del piloto, que se escribe y revisa aparte. Ninguna sesión paga: el arnés se prueba reproduciendo streams escritos a mano.

## Decisiones

1. **Un binario más, `agent-eval`, al lado de los otros**, sin biblioteca compartida (la regla de copiar de revision-eval, `openspec/changes/archive/2026-10-21-revision-eval/design.md:18`) y sin dependencias nuevas (P13 de la exploración).
2. **Los brazos salen del `setup` de producción.** `arm::plan` arma el `State` de `clawg setup claude-code` en memoria, con los archivos ausentes, y toma de `setup::plan` (`crates/clawg-cli/src/setup.rs:200`) los hooks del `settings.json`, el bloque del protocolo en el idioma de la corrida y la línea del servidor. Los escribe en el `CLAUDE_CONFIG_DIR` de la corrida, y el servidor va por `--mcp-config` con `--strict-mcp-config` en vez de `claude mcp add`: armar un brazo no pide `claude` en el PATH. Se descartó correr `setup` contra el home de la corrida: registra el servidor con `claude` (`setup.rs:122`) y escribe copias en `backups/setup` del perfil.
3. **Brazos `a0`, `a2`, `a3`, `a3-eager`, `a4` y `b`.** Los del núcleo y el piloto de P1 y P15, A2 y el brazo B de M3 (A3 sin `search_memory`, `read_memory` ni `list_memories`). A1 y A5, sólo de diagnóstico, no se definen hasta que una pregunta los pida.
4. **La siembra es un export `clawg-memories`** (P2, opción a) que `clawg import --from clawg` carga antes de la sesión, con la fecha de cada memoria por su edad y el tier que el store le daría. Para eso `eval_api` expone `earned_tier`, que llama a `compute_tier` (`crates/clawg-core/src/memory/tier.rs:22`) desde el tier 3 de un insert: copiar la regla dejaría a la siembra desalineada con el store en el primer cambio. A4 recibe los mismos textos en un `MEMORY.md`.
5. **El stream se lee con tolerancia.** El formato es del cliente y cambia con sus versiones (riesgo de P13): una línea de un tipo desconocido se cuenta, una que no es JSON se reporta por número, y nada falla. El texto de un evento de hook se toma de todas sus cadenas, sin depender del nombre del campo.
6. **La corrida en seco usa el mismo camino que una real.** `run --replay` prepara la corrida y lanza `agent-eval replay-client STREAM` en el directorio de trabajo, con las variables del brazo y la salida capturada en `stream.jsonl`, como se lanzaría `claude`. El cliente reproducido aplica los `Write` y `Edit` del stream (sólo rutas dentro del directorio), así que los checks sobre archivos se prueban también. Se descartó un script de shell: no corre igual en Windows.
7. **La rúbrica es sólo lo determinista** (P11): cada criterio es una regla de `protocol.md` que se comprueba sin juez. El idioma es el del perfil con `language::detect`; «nombra un identificador» usa una heurística propia del binario (ruta, `::`, `snake_case`, `camelCase`, nombre de archivo, comillas invertidas) en vez de exponer `identifier_tokens`, que hoy no hace falta para decidir nada.
8. **Los nombres del cliente se escriben como el cliente los documenta y los confirma la primera sesión real:** `--include-hook-events`, `--effort`, `--max-budget-usd`, `CLAUDE_CODE_DISABLE_AUTO_MEMORY`, `ENABLE_TOOL_SEARCH` y el lugar de `MEMORY.md`. Es una de las preguntas del piloto (P15).

## Alcance

- `crates/clawg-eval/src/bin/agent-eval/`: `task.rs` (lectura, validación, trazadores filtrados en el fixture), `arm.rs`, `seed.rs`, `stream.rs`, `score.rs`, `rubric.rs`, `replay.rs`, `run.rs` y `main.rs` con `tasks`, `prepare`, `run --replay`, `score` y `replay-client`.
- `eval_api::earned_tier`.
- `crates/clawg-eval/data/agent-tasks/`: dos tareas de ejemplo (`"example": true`, una K1 y una N) sobre un fixture mínimo, y tres streams escritos a mano.
- `crates/clawg-eval/tests/agent_eval.rs`: la corrida en seco, que CI corre con la suite.
- `docs/reference/testing.md`.

## No-alcance

- **Lanzar `claude` o un daemon.** El corredor real (arrancar el daemon de la corrida, `clawg import`, la sesión, `clawg stop`) usa lo que `prepare` deja en `run.json` y entra con el piloto, que pide autorización de gasto.
- Las seis tareas del piloto y su fixture: se escriben y se revisan aparte.
- La auditoría de trazadores contra el índice de docs (necesita un daemon), el juez de la rúbrica, la estadística entre corridas (McNemar por tarea, k repeticiones) y el informe de campaña.
- Las tareas R con `--resume` y las de M3.

## Cómo se mide el éxito

Tests que hoy no existen y corren en la suite sin modelo, red, daemon ni `claude`: los de cada módulo y `agent_eval.rs`, donde la tarea K1 con su stream de A3 sale `success` con el trazador `used`, la misma con el de A0 sale `failure` con `never_reached`, el `Edit` reproducido de la tarea N cambia el archivo que leen los checks, el save de ese stream pasa todos los criterios que aplican, `score` reproduce el resultado y una corrida nunca escribe sobre otra. La suite y los dos clippy pasan.

## Datos y vuelta atrás

No cambia nada guardado ni el binario que se distribuye (`clawg-eval` no se publica; `earned_tier` sólo existe con la feature `eval`). Nada lee ni escribe un perfil: cada corrida vive en el directorio que se le da. Para volver atrás se revierten los commits.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| Un stream real no se parece a los escritos a mano | El lector tolera lo desconocido y cuenta líneas desconocidas e inválidas en `result.json`; los streams del piloto reemplazan a los de ejemplo como fixtures del parser |
| Un nombre de flag o variable del cliente está mal | `prepare` deja la línea de comando en `run.json` para revisarla antes de gastar; la primera sesión real lo confirma (decisión 8) |
| En Windows clawg toma el perfil del sistema y no `HOME` (`crates/clawg-core/src/paths.rs:163-169`): una corrida real usaría el perfil del usuario | El corredor real no existe todavía; cuando entre, en Windows MUST usar un build `testing` con `CLAWG_TEST_HOME` o negarse |
