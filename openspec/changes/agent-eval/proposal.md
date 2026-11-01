# agent-eval — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (1, 5) | L por fases; esta propuesta cubre el arnés y su corredor, que no gastan (M) | — | ver [`state.yaml`](state.yaml) |

## Problema

`rag-eval` mide qué se recupera y `--agent-view` qué texto le llega al agente, pero nada mide qué hace una sesión de un agente con eso ([exploración](exploration.md), «Tesis»). Cada cambio que tocó lo que ve el agente se verificó con una sesión manual de Claude Code, que no se repite igual. Antes de gastar una sola sesión paga hace falta el instrumento: leer tareas, armar cada brazo, leer el stream del cliente y puntuarlo sin juez. Hoy no existe ninguna de esas piezas en `crates/clawg-eval` (`Cargo.toml` declara cinco binarios y ninguno lanza ni lee un cliente).

## Objetivo

Medición (1, 5): el arnés de la fase 0 del [roadmap](exploration.md#roadmap), con el corredor real, sin el fixture del piloto, que se escribe y revisa aparte. Ninguna sesión paga: el arnés y el corredor se prueban reproduciendo streams escritos a mano, el corredor contra un daemon de verdad con el modelo falso.

## Decisiones

1. **Un binario más, `agent-eval`, al lado de los otros**, sin biblioteca compartida (la regla de copiar de revision-eval, `openspec/changes/archive/2026-10-21-revision-eval/design.md:18`) y sin dependencias nuevas (P13 de la exploración).
2. **Los brazos salen del `setup` de producción.** `arm::plan` arma el `State` de `clawg setup claude-code` en memoria, con los archivos ausentes, y toma de `setup::plan` (`crates/clawg-cli/src/setup.rs:200`) los hooks del `settings.json`, el bloque del protocolo en el idioma de la corrida y la línea del servidor. Los escribe en el `CLAUDE_CONFIG_DIR` de la corrida, y el servidor va por `--mcp-config` con `--strict-mcp-config` en vez de `claude mcp add`: armar un brazo no pide `claude` en el PATH. Se descartó correr `setup` contra el home de la corrida: registra el servidor con `claude` (`setup.rs:122`) y escribe copias en `backups/setup` del perfil.
3. **Brazos `a0`, `a2`, `a3`, `a3-eager`, `a4` y `b`.** Los del núcleo y el piloto de P1 y P15, A2 y el brazo B de M3 (A3 sin `search_memory`, `read_memory` ni `list_memories`). A1 y A5, sólo de diagnóstico, no se definen hasta que una pregunta los pida.
4. **La siembra es un export `clawg-memories`** (P2, opción a) que `clawg import --from clawg` carga antes de la sesión, con la fecha de cada memoria por su edad y el tier que el store le daría. Para eso `eval_api` expone `earned_tier`, que llama a `compute_tier` (`crates/clawg-core/src/memory/tier.rs:22`) desde el tier 3 de un insert: copiar la regla dejaría a la siembra desalineada con el store en el primer cambio. A4 recibe los mismos textos en un `MEMORY.md`.
5. **El stream se lee con tolerancia.** El formato es del cliente y cambia con sus versiones (riesgo de P13): una línea de un tipo desconocido se cuenta, una que no es JSON se reporta por número, y nada falla. El texto de un evento de hook se toma de todas sus cadenas, sin depender del nombre del campo.
6. **La corrida en seco usa el mismo camino que una real.** `run --replay` prepara la corrida y lanza `agent-eval replay-client STREAM` en el directorio de trabajo, con las variables del brazo y la salida capturada en `stream.jsonl`, como se lanzaría `claude`. El cliente reproducido aplica los `Write` y `Edit` del stream (sólo rutas dentro del directorio), así que los checks sobre archivos se prueban también. Se descartó un script de shell: no corre igual en Windows.
7. **La rúbrica es sólo lo determinista** (P11): cada criterio es una regla de `protocol.md` que se comprueba sin juez. El idioma es el del perfil con `language::detect`; «nombra un identificador» usa una heurística propia del binario (ruta, `::`, `snake_case`, `camelCase`, nombre de archivo, comillas invertidas) en vez de exponer `identifier_tokens`, que hoy no hace falta para decidir nada.
8. **Los nombres del cliente se escriben como el cliente los documenta y los confirma la primera sesión real:** `--include-hook-events`, `--effort`, `--max-budget-usd`, `CLAUDE_CODE_DISABLE_AUTO_MEMORY`, `ENABLE_TOOL_SEARCH` y el lugar de `MEMORY.md`. Es una de las preguntas del piloto (P15).
9. **El corredor ejecuta lo que `prepare` deja en `run.json`.** El plan suma el daemon de la corrida (`clawg serve --profile P --port 0`, con el puerto efímero para no quitarle el del perfil a un daemon del usuario, y el `daemon.json` que publica), y `run.json` nombra el cliente que juega la sesión (`claude` o el stream de una reproducción), así que lo que se revisa antes de gastar es lo que corre y `score --run` puntúa sin adivinar. El daemon cuenta como listo cuando su `daemon.json` nombra su pid, que publica una vez que escucha (`crates/clawg-cli/src/daemon/serve.rs:198-210`); se descartó sondear con `clawg status`, que lanza un proceso por intento y sin daemon lee el esquema del store (`crates/clawg-cli/src/main.rs:159-161`, `crates/clawg-core/src/memory/backup.rs:567-570`), mientras que el pid del hijo en `daemon.json` prueba además que el que escucha es el de la corrida. Después espera a que `clawg status` diga que el modelo de ese daemon está listo, a lo sumo 15 minutos porque el primer arranque en el home de la corrida lo baja: un import real pide el embedder (`ready_embedder`, `crates/clawg-cli/src/daemon/api.rs:1468-1475`) y antes responde `503 embedder_not_ready`, y una sesión que empezara antes mediría la carga del modelo. Una carga que falló sin reintento, o un daemon que termina mientras tanto, cortan la corrida. `clawg stop` corre también si la siembra o el cliente fallaron, y un hilo cosecha al daemon apenas termina, porque en Unix un zombi haría esperar a `stop` sus 30 s (`crates/clawg-cli/tests/binary.rs:183-186`). Una siembra que no carga no lanza el cliente: un brazo sin su siembra no mide nada y una sesión real gastaría igual.
10. **En Windows el aislamiento se comprueba en el binario.** Allí clawg toma el home del sistema y no `HOME` (`crates/clawg-core/src/paths.rs:181-191`), y sólo un build `testing` lee `CLAWG_TEST_HOME`, que toda corrida pasa con su home. El corredor se niega, antes de preparar nada, a un `--clawg` que no lleva ese nombre: es la misma marca que `release.yml` busca para rechazar un release (`.github/workflows/release.yml:124`). Se descartó preguntarle a clawg por su directorio de datos: todo comando que lo muestra (`status`, `doctor`) lee el store, y en un build que no es `testing` leería el del usuario.
11. **Tope de gasto, modelo, idioma y versión de Claude Code son parámetros de cada corrida**, que `run.json` registra, porque son decisiones abiertas de la [exploración](exploration.md#decisiones-abiertas). Una corrida sin `--replay` lanza `claude` y pide `--max-budget-usd` y `--claude-version`; antes de preparar nada se niega si `claude --version` informa otra versión.

## Alcance

- `crates/clawg-eval/src/bin/agent-eval/`: `task.rs` (lectura, validación, trazadores filtrados en el fixture), `arm.rs`, `seed.rs`, `stream.rs`, `score.rs`, `rubric.rs`, `replay.rs`, `run.rs` y `main.rs` con `tasks`, `prepare`, `run --replay`, `score` y `replay-client`.
- `eval_api::earned_tier`.
- `crates/clawg-eval/data/agent-tasks/`: dos tareas de ejemplo (`"example": true`, una K1 y una N) sobre un fixture mínimo, y tres streams escritos a mano.
- `crates/clawg-eval/tests/agent_eval.rs`: la corrida en seco, que CI corre con la suite.
- El corredor real en `run.rs` y `main.rs`: el daemon de la corrida, `clawg import`, el cliente y `clawg stop`, con lo que `prepare` deja en `run.json`; la negativa en Windows a un `clawg` que no es `testing`; `--claude-version` y la corrida de `claude` con tope de gasto.
- `docs/reference/testing.md`.

## No-alcance

- **Correr `claude` de verdad.** El corredor lo lanza, pero ninguna sesión paga corre en este cambio: la primera entra con el piloto, que pide autorización de gasto.
- Las seis tareas del piloto y su fixture: se escriben y se revisan aparte.
- La auditoría de trazadores contra el índice de docs (necesita un daemon), el juez de la rúbrica, la estadística entre corridas (McNemar por tarea, k repeticiones) y el informe de campaña.
- Las tareas R con `--resume` y las de M3.

## Cómo se mide el éxito

Tests que hoy no existen y corren en la suite sin modelo real, red ni `claude`: los de cada módulo y `agent_eval.rs`, donde la tarea K1 con su stream de A3 sale `success` con el trazador `used`, la misma con el de A0 sale `failure` con `never_reached`, el `Edit` reproducido de la tarea N cambia el archivo que leen los checks, el save de ese stream pasa todos los criterios que aplican, `score` reproduce el resultado y una corrida nunca escribe sobre otra. Los del corredor arrancan el `clawg` `testing` que la suite construye, con el modelo falso: la corrida de A3 deja el store en el home de la corrida con la siembra importada y el daemon detenido, A0 no arranca ningún clawg, un cliente que falla igual detiene el daemon, un daemon que no escucha no lanza el cliente y un `claude` de otra versión se rechaza antes de preparar nada. La suite y los dos clippy pasan.

## Datos y vuelta atrás

No cambia nada guardado ni el binario que se distribuye (`clawg-eval` no se publica; `earned_tier` sólo existe con la feature `eval`). Nada lee ni escribe un perfil: cada corrida vive en el directorio que se le da, y el daemon que arranca guarda su store en el home de la corrida. Para volver atrás se revierten los commits.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| Un stream real no se parece a los escritos a mano | El lector tolera lo desconocido y cuenta líneas desconocidas e inválidas en `result.json`; los streams del piloto reemplazan a los de ejemplo como fixtures del parser |
| Un nombre de flag o variable del cliente está mal | `prepare` deja la línea de comando en `run.json` para revisarla antes de gastar; la primera sesión real lo confirma (decisión 8) |
| En Windows clawg toma el perfil del sistema y no `HOME` (`crates/clawg-core/src/paths.rs:181-191`): una corrida real usaría el perfil del usuario | Toda corrida pasa su home en `CLAWG_TEST_HOME`, y en Windows el corredor se niega a un `clawg` sin esa marca (decisión 10) |
| El daemon de una corrida real baja el modelo a su home, una vez por corrida | `CLAWG_HF_BASE_URL` puede nombrar un espejo local; copiar el modelo a cada home queda para el piloto, si el tiempo de bajada pesa |
| Una corrida real gasta con una versión del cliente que no es la de la campaña | `--claude-version` es obligatorio sin `--replay` y se comprueba antes de preparar nada (decisión 11) |
