# protocol-block — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, y el costo de cada sesión | M | — ; el valor por defecto lo decide la campaña de [agent-eval-sessions](../agent-eval-sessions/proposal.md) (129), con la regla de [design](design.md#d6-la-regla-de-decisión-del-valor-por-defecto) | propuesta; las decisiones abiertas de la [exploración](exploration.md) se cierran con su opción recomendada |

## Problema

1. **El bloque instalado queda viejo y nada lo dice (bug).** `clawg setup claude-code` escribe `protocol::text` entre marcadores en el `CLAUDE.md` del usuario (`block`, `crates/clawg-cli/src/setup.rs:603-607`). Ni `clawg update` ni el daemon lo tocan después, y `clawg doctor` sólo lo compara cuando hay preferencia de idioma (`crates/clawg-cli/src/doctor/mod.rs:185-187`). Desde verdict-on-replace (0.85.0) las instrucciones del servidor dicen «si valen las dos, no hagas nada» (`crates/clawg-cli/src/mcp/protocol.md:24-28`), y en una máquina real el bloque sigue pidiendo juzgar cada cercana como `compatible` o `scoped`. En los transcripts de este repositorio, el 97 % de 1.973 `resolve_conflict` fueron esos dos veredictos (`docs/completed-impl/hipotesis/A1/informe.md`, R1).
2. **El bloque cuesta unos 3.550 tokens por pedido** y en las sesiones cortas del arnés no cambió ningún resultado (`l0` contra `a3`, `openspec/changes/archive/2026-11-13-memory-delivery/verify.md:51`). Repite entera la apertura que ya va en las instrucciones del servidor (`setup.rs:604`, `protocol.rs:33`). No está medido qué pasa en una sesión larga, que es donde el agente busca y guarda.

## Alcance

En `crates/clawg-cli` y en el arnés de `crates/clawg-eval`:

- **El sello.** `setup` escribe, en la línea siguiente al marcador de apertura, `<!-- clawg X.Y.Z -->`, la versión que escribió el bloque, y `protocol=<variante>` cuando la persona eligió una. Los marcadores no cambian, así que un binario anterior lo sigue encontrando, reescribiendo y quitando.
- **La frescura del bloque**, pura: `current` si el texto, sin el sello y sin mirar los finales de línea, es el que este binario escribiría para la variante y el idioma; `newer` si difiere y lo escribió una versión posterior; `stale` en otro caso. Un bloque sin sello es anterior a este cambio.
- **`clawg doctor`** da `protocol_block` siempre que hay un bloque, con preferencia o sin ella: `ok`, `warn` `stale` con la orden de `setup`, `info` `newer` sin orden, o `warn` `unreadable`. Sin bloque, sólo con preferencia da `info` `not_installed`, como hoy.
- **`clawg hook session-start`** de Claude Code suma una línea cuando el bloque está `stale`: que difiere de lo que escribe este clawg, que donde difieran siga las instrucciones del servidor, y que le diga una vez a la persona la orden exacta que lo reescribe. La imprime en cada inicio de sesión, aun sin daemon.
- **`--protocol full|short|off`** en `clawg setup claude-code`: `full` es el bloque de hoy; `short` es una línea y las secciones «Session summary» y «After a compaction» del mismo archivo; `off` deja sólo los marcadores y el sello. Sin la opción, `setup` mantiene la variante que el sello dice que se eligió, y si no se eligió ninguna escribe la de por defecto, que en este cambio es `full`.
- **El brazo `a3-short`** de `agent-eval`: `a3` con el bloque corto que escribe `setup`.
- `docs/reference/commands.md` (`setup`, `doctor`, `hook`) y `docs/reference/testing.md` (el brazo).

## Fuera de alcance

- **Cambiar el valor por defecto.** Queda en `full` hasta que la campaña de 129 corra con la regla fijada en [design](design.md#d6-la-regla-de-decisión-del-valor-por-defecto). Lo aplica un commit propio después de esa campaña.
- **Escribir el `CLAUDE.md` por cuenta propia**: ni el daemon, ni el hook, ni `clawg update` lo reescriben ([design](design.md#d1-detectar-y-avisar-no-reescribir)).
- Las instrucciones del servidor y las descripciones de las herramientas: el contrato del agente (`agent_contract.json`, `agent_contract.es.json`) no cambia, y `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` tienen que pasar sin regenerarse (`CLAWG_BLESS=1`, `crates/clawg-cli/src/mcp/server.rs:3300`, `:3440`).
- El daemon y su API: `crates/clawg-cli/src/daemon/api.rs` no se toca.
- Cursor: no tiene bloque, y su hook no suma la línea.
- `--local` y `--path`: no escriben el bloque (`crates/clawg-cli/src/setup/local.rs:1-4`) y no toman `--protocol`.

## Decisiones

Están en [design.md](design.md): detectar y avisar sin reescribir (D1), el sello con versión y variante (D2), el aviso en cada inicio de sesión (D3), las tres variantes salidas de un mismo archivo (D4), que la variante elegida viva en el sello (D5) y la regla de decisión del valor por defecto (D6).

## Cómo se mide el éxito

**Sin pagar**, con tests que hoy fallan:

- `doctor` sin preferencia y con un bloque viejo da `warn` `stale` con `clawg setup claude-code --profile P` (`doctor_marks_an_old_block_stale_without_a_preference`, `tests/binary.rs`). Hoy no da ningún hallazgo.
- `session-start` con un bloque viejo imprime la línea, con y sin daemon, y no la imprime con un bloque al día, más nuevo, ausente o ilegible (`session_start_says_when_the_block_is_stale`, `hook.rs`).
- `setup --protocol short` escribe el bloque corto, una segunda corrida sin la opción no cambia nada y `doctor` lo da `ok` (`setup.rs`, `tests/binary.rs`).
- El contrato del agente no cambia.

**Con uso real**, después de instalar la versión: en la primera sesión de una máquina con el bloque de antes de 0.85.0, el agente dice la orden; después de correrla, `doctor` da `ok`. Con los scripts de A1 sobre los transcripts de las semanas siguientes, los `resolve_conflict` por save con cercanas bajan de 3,0 a menos de 0,15, el umbral que ya fijó verdict-on-replace ([verify](../archive/2026-11-12-verdict-on-replace/verify.md#pendiente-en-prod)).

**El valor por defecto**, con la campaña de 129 y la regla D6, fijada antes de correr.

## Riesgos

- **El aviso no entra en el presupuesto del hook.** Lo que `session-start` suma tiene que quedar bajo 1.500 bytes ([`specs/hooks`](../../specs/hooks/spec.md) R4), y el peor caso de hoy, con todos los avisos en español, ya anda cerca. La spec sube el tope a 2.000 bytes, que con los 8.000 del contexto sigue dentro de los 10.000 que inyecta el cliente (`SESSION_BUDGET`, `crates/clawg-cli/src/hook.rs:65`).
- **El agente no le dice nada a la persona.** La línea pide decirlo una vez; si no lo hace, `doctor` lo sigue marcando y la sesión igual sigue las instrucciones del servidor, que es lo que importa.
- **Un bloque editado a mano** se ve `stale` y la orden lo reescribe, como ya hace `setup` (R4 de `specs/setup`). El mensaje dice «no es el que escribe este clawg», no «es viejo».
- **La variante elegida se pierde si alguien borra la línea del sello.** El bloque queda sin elección y `setup` escribe la de por defecto: lo mismo que hoy.

## Datos guardados

Ninguno: no cambia `memory.db`, el esquema ni ningún identificador. El único archivo que cambia es el `CLAUDE.md` del usuario, y sólo cuando la persona corre `setup`, con la copia en `backups/setup/` que `setup` ya hace. Para volver atrás: `clawg setup claude-code --protocol full` deja el bloque de hoy, con sólo la línea del sello de más, y un binario anterior lo reescribe sin ella.
