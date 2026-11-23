# summary-on-stop — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, y el criterio de la etapa «la mayoría de las sesiones que guardan dejan su resumen» | M: un hook nuevo, una consulta al daemon y `setup`; no toca `memory.db` | —; se coordina con [conflicts-on-demand](../archive/2026-11-23-conflicts-on-demand/proposal.md) (141), que hace que el resumen llegue entero al arranque, y con [session-lifecycle](../session-lifecycle/exploration.md) (99), dormido, que ve el mismo problema con sesiones en paralelo | exploración |

## Problema

La sesión siguiente arranca sabiendo dónde quedó la anterior sólo si la anterior guardó un `session_summary`, y eso depende de que el agente se acuerde. El conteo de [session-usage](../archive/2026-11-11-session-usage/proposal.md) (121) dice que sólo 27 de las 122 sesiones con prompts guardaron uno. conflicts-on-demand arregla que el resumen se corte al llegar; este cambio arregla que no se escriba.

Ningún hook de clawg corre cuando el agente termina un turno: `clawg hook` contesta `SessionStart`, `UserPromptSubmit`, `SessionEnd`, `PostToolUse` y `PostToolUseFailure` (`crates/clawg-cli/src/cli.rs:962-974`), y `setup` instala esos cinco (`crates/clawg-cli/src/setup.rs:59`, `crates/clawg-cli/src/setup/local.rs:258-260`). `SessionEnd` llega tarde para esto: el agente ya no puede escribir.

## Lo que ya existe

- **El daemon ya sabe qué guardó cada sesión, por tipo.** Cada save aceptado suma `Activity::Save(tipo)` a la sesión vigente (`crates/clawg-cli/src/daemon/api/memories.rs:861-882`, `crates/clawg-core/src/metrics/sessions.rs:322-325`), y el reporte ya separa las sesiones que guardaron un `session_summary` (`crates/clawg-core/src/metrics/session_report.rs:203`). Lo que falta es preguntarlo por un `session_id` desde un hook.
- **La sesión de un save se deduce por agente y proyecto**, de la última que registró un hook (`crates/clawg-cli/src/daemon/sessions.rs:82-85`). Con dos sesiones del mismo proyecto en paralelo, los saves de una se cuentan en la otra: es el caso de session-lifecycle (99).
- **El cliente ya habla con `/v1/sessions` y `/v1/sessions/end`** (`crates/clawg-cli/src/client.rs:1285-1299`).
- **Claude Code:** el hook `Stop` recibe `session_id`, `transcript_path` y `stop_hook_active`. Salir con 2, o contestar `{"decision": "block", "reason": "…"}`, hace seguir al agente con ese texto; un script tiene que salir sin bloquear si `stop_hook_active` es true, y Claude Code corta el hook tras 8 bloqueos seguidos. `"onFailure": "block"` no tiene efecto en `Stop`, así que un hook que falla deja terminar.

## Qué haría

Cuando el agente va a terminar un turno, `clawg hook stop` pregunta al daemon qué guardó esa sesión. Si guardó algo y ningún `session_summary`, y todavía no se le pidió en esa sesión, contesta `block` con un pedido corto: que guarde el resumen con sus secciones y termine. En cualquier otro caso, o si el daemon no contesta a tiempo, no imprime nada y el agente termina.

## Enfoques

- **A. Preguntar al daemon (recomendado).** Un endpoint de sólo lectura con los saves por tipo de un `session_id`, y una marca en memoria de que ya se pidió. Usa el conteo que ya existe y no lee archivos del cliente.
- **B. Leer el transcript.** El hook recorre `transcript_path` buscando llamadas a `save_memory`. No toca el daemon, pero depende del formato del transcript, que no es un contrato, y de los nombres de herramienta del puente.

## Decisiones abiertas

1. **Una vez por sesión o una vez por turno con saves nuevos.** Una por sesión es lo pedido; un resumen viejo de una sesión larga queda atrás, pero el pedido repetido cuesta un turno cada vez.
2. **Qué cuenta como «guardó algo».** Todo save, o sólo los que no son `session_summary` ni `tool_error`. Una sesión que sólo registró un error no tiene qué resumir.
3. **Cursor.** Tiene un evento `stop` con otra forma; ¿entra ahora o sólo Claude Code, como hizo repeated-errors (131)?
4. **El texto del pedido**, que es contrato que el modelo lee: se fija en un archivo como los de `agent_contract.json`.

## Hecho, en borrador

- Una sesión que guardó un hecho y ningún resumen recibe un `block` en su primer `Stop`.
- La misma sesión no lo recibe en un segundo `Stop`, ni con `stop_hook_active`.
- Una sesión que no guardó nada, o que ya guardó su resumen, no recibe nada.
- Con el daemon apagado o lento, el hook sale sin imprimir dentro de su plazo.
- `setup` instala `Stop` en los dos alcances y `doctor` avisa cuando falta.
- Después de publicar, en local: el reporte de `clawg metrics sessions` sube la fracción de sesiones con resumen frente a 27 de 122.
