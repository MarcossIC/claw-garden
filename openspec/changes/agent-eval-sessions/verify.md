# agent-eval-sessions — Verificación

Veredicto: **cumple lo que la suite puede comprobar**. Quedan fuera de esta verificación el CI de Linux y Windows y la campaña, que es local y paga.

## Qué se corrió

En Windows, sobre el último commit de código, con `target-test` y la copia del árbol sin cambios sin commitear en `crates/`:

- `cargo test --workspace --target-dir target-test`: pasa.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -D warnings`: pasa.
- `cargo clippy -p clawg-cli --target-dir target-test -- -D warnings`: pasa.
- `agent-eval tasks`: sale con 0. Da válidas las catorce tareas, con las seis nuevas (`requests-timeout-decision-relay` con `sessions=2`, `requests-redact-decision-revised` con `sessions=3`, `requests-history-owner-convention`, `shelf-eza-missing` con `sessions=3`, y los ejemplos `example-relay-port` y `example-missing-command` con `sessions=2`), ninguna con un trazador en un archivo de su fixture, y lista los dos ruidos: `noise rules=15` y `noise-300 rules=300`.

El cambio no toca el ranking, así que no corre `rag-eval`. Ninguna corrida fue paga: no hubo `claude` ni `ANTHROPIC_API_KEY`.

## Requisitos

| Requisito | Dónde | Veredicto |
| --- | --- | --- |
| AS1 orden del `MEMORY.md` | `seed::native_memory` (commit anterior a esta tanda) | Cumple |
| AS2 tarea de varias sesiones | `task.rs`: `sessions`, `Task::prompts`, trazadores de sesión | Cumple |
| AS3 fallos de una herramienta, shell, `E` | `Check::ToolFailedCount`, `Task::shell`, `Task::missing_commands`, `arm::plan` | Cumple |
| AS4 plan de varias sesiones | `Shape`, `RunPlan::later`, `run::session_files` | Cumple |
| AS5 corredor de varias sesiones | `run::play_session`; una sesión que falla no deja empezar la siguiente y la corrida se puntúa con las que corrieron | Cumple |
| AS6 bifurcación | `run::fork_steps`, `Fork`, `save_state` y `restore_state`; ramas en `joined/` y `split/` | Cumple |
| AS7 corrida en seco contra el servidor | `replay::server_calls`, `replay::with_results`; directorio de streams, también el de una bifurcación | Cumple |
| AS8 tokens de cada pedido y de cada llamada | `Request`, `ToolCall::added_tokens`; ahora también `Request::cache_read` | Cumple |
| AS9 puntaje por sesión y camino de un hecho | `score::score_session`, `score::score`, `run::evaluate`, `run::facts`, `run::overhead`, `seed::native_loaded` | Cumple, con un matiz, ver abajo |
| AS10 reporte, costos y llamadas | `report::aggregate`, `pairs`, `costs`, `calls`; `agent-eval calls` | Cumple, con dos matices, ver abajo |
| AS11 ruido de una corrida | `--noise`, `task::load_noise`, `noise-300.json`, `ArmError::StartupTooLong` | Cumple |
| AS12 tareas nuevas y fuentes | cuatro tareas, `psf-requests` (64 archivos, 601855 bytes, 17811 líneas, tal como en la fuente), `shelf-notes`, `SOURCES.md` | Cumple |

## Desvíos y matices

1. **El escenario «un hecho que no se guardó» de AS9.** Dice que en la corrida `joined` el trazador sale `reached_not_used`, traído por `context`. La regla de clasificación de la misma sección da `not_saved` a todo hecho que ninguna sesión escribió en una memoria, sea cual sea la forma. Se implementó la regla: un hecho que sólo está en el prompt de una sesión anterior de una corrida continua sale `not_saved`, y su camino por sesión sí lo muestra traído por `context`. El escenario es el que hay que corregir al archivar.
2. **La tabla de `report` cambia de columnas.** AS10 pide que los tests de AE8 y MD7 sigan pasando. Sus aserciones sobre líneas exactas se actualizaron para las columnas nuevas (`shape` después del brazo, y al final las sesiones, el contexto, el sobrecosto y las clases de hecho). La semántica con resultados de una sesión no cambia.
3. **`report` no suma dos veces la primera sesión de una bifurcación.** Las dos ramas la llevan en su `result.json`. Los totales y `costs` la cuentan una vez, en la rama continua. Los promedios por fila sí son los de la corrida entera de cada forma.
4. **El ahorro de un par** usa el costo de la segunda sesión de cada rama, porque la primera es la misma. AS10 no lo precisa.
5. **El stream de una bifurcación en seco** es un directorio aparte, `replays/example-relay-port.a3.both/`, porque el directorio de la forma `split` ya tiene su `session-2.jsonl` y un directorio con un stream de más se rechaza. Los streams del ejemplo buscan por `STAGING_PORT` y `RETRY_BUDGET`, no por `48213` ni `7719`: con el modelo falso una búsqueda sólo encuentra por un identificador con letras o por el texto idéntico (nota en el [diseño](design.md), D6).
6. **Una corrida de varias sesiones que falla** deja un `result.json` con las sesiones que corrieron y no terminada; una de una sesión que falla sigue sin dejarlo, como antes.
7. **`docs/make.bat` de `psf-requests` conserva sus CRLF.** `.gitattributes` lo deja con `-text` para que el fixture quede tal como en la fuente, y `the_task_data_keeps_lf_line_endings_on_every_system` salta los `.bat`.
8. **Los `Session` de un `result.json` ganan `request_tokens` y `first_request_cache_read`**, con valor por omisión: los resultados anteriores se siguen leyendo.

## Pendiente en local

- El CI en verde en Linux y Windows.
- La campaña: fase 0 (todas las tareas nuevas en seco con el modelo real), fase 1 (humo y canarios, que es una puerta) y las fases 2 a 4, con la regla de decisión de la [propuesta](proposal.md#la-regla-de-decisión) copiada sin cambios. Lo que el arnés no comprueba y sólo una sesión paga muestra: que `--continue` conserve el contexto en `-p`, que la memoria nativa escriba en `-p`, el modelo de carga de 200 líneas o 25.000 bytes del `MEMORY.md` y el costo real de las sesiones largas.
- Al archivar: AS1 reemplaza el orden del `MEMORY.md` en MD3 de `openspec/specs/eval/spec.md`, y AS2 a AS12 pasan a esa sección, con el escenario de AS9 corregido.
